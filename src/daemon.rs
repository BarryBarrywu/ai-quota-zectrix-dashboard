use crate::{
    antigravity,
    codex::{CodexFailure, CodexObserver},
    model::{AntigravityQuota, CodexQuota, Dashboard, Radar, Source},
    publisher::{
        CLOUD_BASE, Cloud, PublishOutcome, Result, Settings, last_success, publish_changed,
        read_optional, save,
    },
    radar::{Provider, RadarObserver},
    setup::keychain_key,
};
use chrono::{DateTime, FixedOffset, Local, TimeZone};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};

#[derive(Default, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Current,
    Stale,
    #[default]
    Unavailable,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Reason {
    NoObservation,
    DaemonStopped,
    Transport,
    Rejected,
    InvalidQuota,
    NoWeeklyWindow,
    SnapshotMissingOrInvalid,
    SnapshotExpired,
    RateLimited,
    RadarReadFailed,
    RadarHttpError,
    RadarInvalidStatus,
    RadarResponseTooLarge,
    CacheWriteFailed,
    NotConfigured,
    KeychainUnavailable,
    UploadFailed,
    Throttled,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
struct Entry {
    status: Status,
    reason: Option<Reason>,
}
impl Default for Entry {
    fn default() -> Self {
        Self {
            status: Status::Unavailable,
            reason: Some(Reason::NoObservation),
        }
    }
}
impl Entry {
    fn observed<T>(source: &Source<T>, reason: Option<Reason>) -> Self {
        let status = match source {
            Source::Current(_) => Status::Current,
            Source::Stale(_) => Status::Stale,
            Source::Unavailable => Status::Unavailable,
        };
        Self { status, reason }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Report {
    codex_quota: Entry,
    antigravity_quota: Entry,
    codex_radar: Entry,
    antigravity_radar: Entry,
    publisher: Entry,
}

pub fn diagnostics(dir: &Path) -> Result<String> {
    let mut report: Report = read_optional(&dir.join("diagnostics.json"))
        .ok()
        .flatten()
        .unwrap_or_default();
    let running = fs::File::open(dir.join("run.lock"))
        .is_ok_and(|file| matches!(file.try_lock(), Err(fs::TryLockError::WouldBlock)));
    if !running {
        let stopped = |available| Entry {
            status: if available {
                Status::Stale
            } else {
                Status::Unavailable
            },
            reason: Some(if available {
                Reason::DaemonStopped
            } else {
                Reason::NoObservation
            }),
        };
        report.codex_quota = stopped(
            read_optional::<CodexQuota>(&dir.join("codex-quota.json"))
                .ok()
                .flatten()
                .is_some(),
        );
        report.antigravity_quota = stopped(
            read_optional::<AntigravityQuota>(&dir.join("antigravity-quota.json"))
                .ok()
                .flatten()
                .is_some(),
        );
        let [codex, agy] = radar_pair(dir);
        let now = Local::now().timestamp();
        report.codex_radar = stopped(codex.view(now).value().is_some());
        report.antigravity_radar = stopped(agy.view(now).value().is_some());
        report.publisher = stopped(last_success(dir).ok().flatten().is_some());
    }
    Ok(serde_json::to_string_pretty(&report)?)
}

pub struct Observers {
    dir: PathBuf,
    codex: CodexObserver,
    snapshot: Option<PathBuf>,
    quotas: (Source<CodexQuota>, Source<AntigravityQuota>),
    radars: [RadarObserver; 2],
    quota_due: i64,
    report: Report,
}

impl Observers {
    pub fn new(
        dir: &Path,
        program: &Path,
        snapshot: Option<PathBuf>,
        radars: [RadarObserver; 2],
    ) -> Self {
        let mut codex = CodexObserver::new(program);
        if let Ok(Some(value)) = read_optional(&dir.join("codex-quota.json")) {
            codex.restore_cache(value);
        }
        let agy: Option<AntigravityQuota> = read_optional(&dir.join("antigravity-quota.json"))
            .ok()
            .flatten();
        let agy = agy.filter(|q| {
            [&q.gemini, &q.claude_gpt]
                .into_iter()
                .all(|w| w.remaining_percent <= 100 && w.resets_at > 0)
        });
        Self {
            dir: dir.into(),
            codex,
            snapshot,
            quotas: (
                Source::Unavailable,
                agy.map(Source::Stale).unwrap_or_default(),
            ),
            radars,
            quota_due: i64::MIN,
            report: Report::default(),
        }
    }
    pub fn local(dir: &Path, program: &Path) -> Self {
        Self::new(
            dir,
            program,
            std::env::var_os("AI_QUOTA_STATUSLINE_PATH").map(PathBuf::from),
            radar_pair(dir),
        )
    }

    fn poll(&mut self, now: i64, persist: bool) -> Result<()> {
        let [codex_radar, agy_radar] = &mut self.radars;
        thread::scope(|scope| -> Result<()> {
            if now >= codex_radar.next_poll_at() {
                scope.spawn(move || codex_radar.poll(now));
            }
            if now >= agy_radar.next_poll_at() {
                scope.spawn(move || agy_radar.poll(now));
            }
            if now >= self.quota_due {
                self.quota_due = now.saturating_add(600);
                let quota = self.codex.poll();
                let why = self.codex.last_failure().map(|r| match r {
                    CodexFailure::Transport => Reason::Transport,
                    CodexFailure::Rejected => Reason::Rejected,
                    CodexFailure::NoWeeklyWindow => Reason::NoWeeklyWindow,
                    CodexFailure::InvalidQuota => Reason::InvalidQuota,
                });
                self.report.codex_quota = Entry::observed(&quota, why);
                if persist {
                    if let Source::Current(ref q) = quota {
                        save(&self.dir.join("codex-quota.json"), &serde_json::to_vec(q)?)?;
                    } else if matches!(why, Some(Reason::NoWeeklyWindow)) {
                        match fs::remove_file(self.dir.join("codex-quota.json")) {
                            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                                return Err(e.into());
                            }
                            _ => (),
                        }
                    }
                }
                self.quotas.0 = quota;
                let (agy, reason) =
                    antigravity::observe(self.snapshot.as_deref(), self.quotas.1.value());
                let why = reason.map(|s| match s {
                    antigravity::SnapshotFailure::Expired => Reason::SnapshotExpired,
                    antigravity::SnapshotFailure::MissingOrInvalid => {
                        Reason::SnapshotMissingOrInvalid
                    }
                });
                self.report.antigravity_quota = Entry::observed(&agy, why);
                if persist
                    && !matches!(why, Some(Reason::SnapshotMissingOrInvalid))
                    && let Some(q) = agy.value()
                {
                    save(
                        &self.dir.join("antigravity-quota.json"),
                        &serde_json::to_vec(q)?,
                    )?;
                }
                self.quotas.1 = agy;
            }
            Ok(())
        })
    }

    fn dashboard(&mut self, now: DateTime<FixedOffset>) -> Dashboard {
        let stamp = now.timestamp();
        let [c, a] = &self.radars;
        let (codex_radar, antigravity_radar) = (c.view(stamp), a.view(stamp));
        let radar_entry = |r: &RadarObserver, source: &Source<Radar>| {
            let why = match r.last_error() {
                Some("rate limited") => Some(Reason::RateLimited),
                Some("http error") => Some(Reason::RadarHttpError),
                Some("invalid status") => Some(Reason::RadarInvalidStatus),
                Some("response too large") => Some(Reason::RadarResponseTooLarge),
                Some("cache write failed") => Some(Reason::CacheWriteFailed),
                Some(_) => Some(Reason::RadarReadFailed),
                None if !matches!(source, Source::Current(_)) => Some(Reason::NoObservation),
                None => None,
            };
            Entry::observed(source, why)
        };
        self.report.codex_radar = radar_entry(c, &codex_radar);
        self.report.antigravity_radar = radar_entry(a, &antigravity_radar);
        Dashboard {
            now,
            screen_updated_at: last_success(&self.dir)
                .ok()
                .flatten()
                .and_then(|t| Local.timestamp_opt(t, 0).single())
                .map(|t| t.fixed_offset()),
            codex: self.quotas.0.clone(),
            antigravity: self.quotas.1.clone(),
            codex_radar,
            antigravity_radar,
        }
    }

    pub fn live(mut self) -> Result<Dashboard> {
        self.radars = self.radars.map(RadarObserver::read_only);
        self.poll(Local::now().timestamp(), false)?;
        Ok(self.dashboard(Local::now().fixed_offset()))
    }
}

fn radar_pair(dir: &Path) -> [RadarObserver; 2] {
    [
        RadarObserver::new(Provider::Codex, dir.join("codex-radar.json")),
        RadarObserver::new(Provider::Antigravity, dir.join("antigravity-radar.json")),
    ]
}

fn display_deadline(d: &Dashboard) -> i64 {
    let now = d.now.timestamp();
    let mut next = d
        .now
        .date_naive()
        .succ_opt()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .and_then(|local| Local.from_local_datetime(&local).earliest())
        .map_or(now.saturating_add(86400), |t| t.timestamp());
    for w in d.codex.value().map(|q| &q.weekly).into_iter().chain(
        d.antigravity
            .value()
            .into_iter()
            .flat_map(|q| [&q.gemini, &q.claude_gpt]),
    ) {
        let remaining = w.resets_at.saturating_sub(now);
        if remaining >= 60 {
            let unit = if remaining >= 86400 { 3600 } else { 60 };
            next = next.min(now.saturating_add(remaining % unit + 1));
        }
    }
    for r in [&d.codex_radar, &d.antigravity_radar]
        .into_iter()
        .filter_map(Source::value)
    {
        match r {
            Radar::Scheduled { starts_at, .. } if *starts_at > now => next = next.min(*starts_at),
            Radar::Confirmed { announced_at, .. } => {
                next = next.min(now + 60 - (now - announced_at).rem_euclid(60))
            }
            _ => (),
        }
    }
    next
}

pub fn run_local(observers: Observers) -> Result<()> {
    run(
        observers,
        || {
            keychain_key()?
                .map(|key| Cloud::new(CLOUD_BASE, &key))
                .transpose()
        },
        || Local::now().fixed_offset(),
        Duration::from_secs,
    )
}

pub fn run(
    mut observers: Observers,
    cloud: impl Fn() -> Result<Option<Cloud>>,
    now: impl Fn() -> DateTime<FixedOffset>,
    wait_duration: impl Fn(u64) -> Duration,
) -> Result<()> {
    let dir = observers.dir.clone();
    fs::create_dir_all(&dir)?;
    let lock = fs::File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("run.lock"))?;
    lock.try_lock().map_err(|_| "companion already running")?;
    let (stop, stopped) = mpsc::channel();
    ctrlc::set_handler(move || {
        let _ = stop.send(());
    })?;
    let mut retry = i64::MIN;
    loop {
        observers.poll(now().timestamp(), true)?;
        if stopped.try_recv().is_ok() {
            break;
        }
        let d = observers.dashboard(now());
        let stamp = d.now.timestamp();
        if stamp >= retry {
            retry = i64::MIN;
            let result = Settings::load(&dir)
                .map_err(|_| Reason::NotConfigured)
                .and_then(|s| {
                    let c = cloud()
                        .map_err(|_| Reason::KeychainUnavailable)?
                        .ok_or(Reason::KeychainUnavailable)?;
                    publish_changed(&dir, &c, &s, &d).map_err(|_| Reason::UploadFailed)
                });
            let reason = match result {
                Ok(PublishOutcome::Published | PublishOutcome::Unchanged) => None,
                Ok(PublishOutcome::Deferred { until }) => {
                    retry = until;
                    Some(Reason::Throttled)
                }
                Err(r) => {
                    retry = stamp.saturating_add(if matches!(r, Reason::NotConfigured) {
                        600
                    } else {
                        60
                    });
                    Some(r)
                }
            };
            observers.report.publisher = Entry {
                status: if reason.is_none() {
                    Status::Current
                } else if d.screen_updated_at.is_some() {
                    Status::Stale
                } else {
                    Status::Unavailable
                },
                reason,
            };
        }
        save(
            &dir.join("diagnostics.json"),
            &serde_json::to_vec(&observers.report)?,
        )?;
        let mut deadline = observers.quota_due.min(display_deadline(&d));
        for r in &observers.radars {
            deadline = deadline
                .min(r.next_poll_at())
                .min(r.next_expiry(stamp).unwrap_or(i64::MAX));
        }
        if retry > stamp {
            deadline = deadline.min(retry);
        }
        let seconds = deadline.saturating_sub(now().timestamp()).max(0) as u64;
        if stopped.recv_timeout(wait_duration(seconds)).is_ok() {
            break;
        }
    }
    Ok(())
}
