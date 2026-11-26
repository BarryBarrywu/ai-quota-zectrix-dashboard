use crate::model::{Radar, ResetKind, Scope, Source};
use chrono::DateTime;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs, io::Read, net::SocketAddr, path::PathBuf, time::Duration};

const HOUR: i64 = 3600;
const DAY: i64 = 24 * HOUR;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    Codex,
    Antigravity,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Events {
    schedule: Option<Radar>,
    confirmation: Option<Radar>,
}

#[derive(Serialize, Deserialize)]
struct Cache {
    provider: Provider,
    events: Events,
    etag: Option<String>,
    retry_after: Option<i64>,
}

pub struct RadarObserver {
    provider: Provider,
    endpoint: String,
    path: PathBuf,
    read_only: bool,
    events: Events,
    etag: Option<String>,
    retry_after: Option<i64>,
    checked: Option<i64>,
    next_poll: i64,
    error: Option<&'static str>,
}

impl RadarObserver {
    pub fn new(provider: Provider, path: PathBuf) -> Self {
        let host = match provider {
            Provider::Codex => "didcodexreset.com",
            Provider::Antigravity => "agyreset.com",
        };
        let cached = fs::File::open(&path)
            .ok()
            .and_then(|file| serde_json::from_reader::<_, Cache>(file.take(4096)).ok())
            .filter(|cache| cache.provider == provider);
        let (events, etag, retry_after) = cached
            .map(|c| (c.events, c.etag, c.retry_after))
            .unwrap_or_default();
        Self {
            provider,
            endpoint: format!("https://{host}/openapi/v1/status"),
            path,
            read_only: false,
            events,
            etag,
            retry_after,
            checked: None,
            next_poll: retry_after.unwrap_or(i64::MIN),
            error: retry_after.map(|_| "rate limited"),
        }
    }

    pub fn loopback(
        provider: Provider,
        path: PathBuf,
        addr: SocketAddr,
    ) -> Result<Self, &'static str> {
        if !addr.ip().is_loopback() {
            return Err("loopback required");
        }
        let mut observer = Self::new(provider, path);
        observer.endpoint = format!("http://{addr}/openapi/v1/status");
        Ok(observer)
    }

    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    pub fn view(&self, now: i64) -> Source<Radar> {
        let radar = match (&self.events.schedule, &self.events.confirmation) {
            (Some(r @ Radar::Scheduled { ends_at, .. }), _) if now < *ends_at => r.clone(),
            (_, Some(r @ Radar::Confirmed { announced_at, .. }))
                if *announced_at <= now && now < announced_at.saturating_add(DAY) =>
            {
                r.clone()
            }
            _ if self
                .checked
                .is_some_and(|t| t <= now && now < t.saturating_add(2 * HOUR)) =>
            {
                Radar::NoSchedule
            }
            _ => return Source::Unavailable,
        };
        if self.error.is_some() || self.checked.is_none() {
            Source::Stale(radar)
        } else {
            Source::Current(radar)
        }
    }

    pub fn next_expiry(&self, now: i64) -> Option<i64> {
        match self.view(now).value()? {
            Radar::Scheduled { ends_at, .. } => Some(*ends_at),
            Radar::Confirmed { announced_at, .. } => Some(announced_at.saturating_add(DAY)),
            Radar::NoSchedule => self.checked.map(|t| t.saturating_add(2 * HOUR)),
        }
    }

    pub fn next_poll_at(&self) -> i64 {
        self.next_poll
    }
    pub fn last_error(&self) -> Option<&'static str> {
        self.error
    }
    pub fn poll(&mut self, now: i64) -> Source<Radar> {
        if now >= self.next_poll {
            self.next_poll = now.saturating_add(HOUR);
            self.error = self.fetch(now).err();
        }
        self.view(now)
    }

    fn fetch(&mut self, now: i64) -> Result<(), &'static str> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "http client unavailable")?;
        let mut request = client.get(&self.endpoint).timeout(Duration::from_secs(5));
        if let Some(tag) = &self.etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, tag);
        }
        let response = request.send().map_err(|_| "network error")?;
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            if let Some(deadline) = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| {
                    v.parse::<u64>()
                        .ok()
                        .map(|seconds| {
                            now.saturating_add(i64::try_from(seconds).unwrap_or(i64::MAX))
                        })
                        .or_else(|| {
                            httpdate::parse_http_date(v)
                                .ok()
                                .map(|date| DateTime::<chrono::Utc>::from(date).timestamp())
                        })
                })
            {
                self.next_poll = self.next_poll.max(deadline);
            }
            self.retry_after = Some(self.next_poll);
            self.persist()?;
            return Err("rate limited");
        }
        if response.status() == reqwest::StatusCode::NOT_MODIFIED && self.etag.is_some() {
            self.checked = Some(now);
            if self.retry_after.take().is_some() {
                self.persist()?;
            }
            return Ok(());
        }
        if !response.status().is_success() {
            return Err("http error");
        }
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .filter(|v| v.len() <= 256)
            .map(str::to_owned);
        const MAX_BODY: u64 = 64 * 1024;
        if response.content_length().is_some_and(|n| n > MAX_BODY) {
            return Err("response too large");
        }
        let mut body = Vec::new();
        response
            .take(MAX_BODY + 1)
            .read_to_end(&mut body)
            .map_err(|_| "network error")?;
        if body.len() as u64 > MAX_BODY {
            return Err("response too large");
        }
        self.events = parse(&body, self.provider, now).ok_or("invalid status")?;
        self.retry_after = None;
        self.etag = etag;
        self.checked = Some(now);
        self.persist()
    }

    fn persist(&self) -> Result<(), &'static str> {
        if self.read_only {
            return Ok(());
        }
        let parent = self
            .path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        fs::create_dir_all(parent).map_err(|_| "cache write failed")?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|_| "cache write failed")?;
        let cache = Cache {
            provider: self.provider,
            events: self.events.clone(),
            etag: self.etag.clone(),
            retry_after: self.retry_after,
        };
        serde_json::to_writer(&mut file, &cache).map_err(|_| "cache write failed")?;
        file.persist(&self.path).map_err(|_| "cache write failed")?;
        Ok(())
    }
}

fn timestamp(v: &Value) -> Option<i64> {
    DateTime::parse_from_rfc3339(v.as_str()?)
        .ok()
        .map(|t| t.timestamp())
}

fn scope(record: &Value, provider: Provider) -> Scope {
    if provider == Provider::Codex {
        return Scope::Unknown;
    }
    let scope = &record["scope"];
    match scope["modelScope"].as_str() {
        Some("gemini") => Scope::Gemini,
        Some("3p" | "third_party" | "third-party") => Scope::ClaudeGpt,
        Some("all") => Scope::All,
        Some("named") => Scope::Named,
        None if scope["modelIds"]
            .as_array()
            .is_some_and(|ids| !ids.is_empty()) =>
        {
            Scope::Named
        }
        _ => Scope::Unknown,
    }
}

fn parse(body: &[u8], provider: Provider, now: i64) -> Option<Events> {
    let value: Value = serde_json::from_slice(body).ok()?;
    if value["ok"] != true {
        return None;
    }
    let data = value["data"].as_object()?;
    let scheduled = data.get("nextScheduled")?;
    if !scheduled.is_null() {
        scheduled["scheduleState"].as_str()?;
    }
    let schedule = if scheduled["scheduleState"] == "pending" {
        let window = &scheduled["scheduleWindow"];
        let starts_at = timestamp(&window["startAt"])?;
        let ends_at = timestamp(&window["endAt"])?;
        if ends_at < starts_at {
            return None;
        }
        Some(Radar::Scheduled {
            starts_at,
            ends_at,
            scope: scope(scheduled, provider),
        })
    } else {
        None
    };
    let mut latest = None;
    for (key, kind) in [
        ("latestGlobal", ResetKind::Global),
        ("latestBanked", ResetKind::Banked),
    ] {
        let record = data.get(key)?;
        if record.is_null() {
            continue;
        }
        if record["kind"].as_str()? != "reset_completed" {
            continue;
        }
        let time = ["announcedAt", "effectiveAt", "completedAt"]
            .iter()
            .find_map(|k| timestamp(&record[k]))?;
        if time <= now && latest.as_ref().is_none_or(|(t, _)| time > *t) {
            latest = Some((
                time,
                Radar::Confirmed {
                    kind,
                    announced_at: time,
                    scope: scope(record, provider),
                },
            ));
        }
    }
    Some(Events {
        schedule,
        confirmation: latest.map(|(_, r)| r),
    })
}
