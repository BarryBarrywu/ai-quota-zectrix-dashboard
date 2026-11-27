use crate::model::{AntigravityQuota, GroupReading, QuotaWindow, Source, WindowKind};
use chrono::DateTime;
use serde_json::Value;
use std::{fs::File, io::Read, path::Path, time::UNIX_EPOCH};

const LIMIT: u64 = 2 * 1024 * 1024;

fn json(reader: impl Read) -> Option<Value> {
    let mut bytes = Vec::new();
    reader.take(LIMIT + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= LIMIT).then(|| serde_json::from_slice(&bytes).ok())?
}

#[derive(Default)]
struct Groups {
    windows: [Option<(f64, QuotaWindow)>; 2],
}
impl Groups {
    fn add(&mut self, identity: &str, window: &str, fraction: f64, resets_at: Option<i64>) {
        let identity = identity.to_ascii_lowercase();
        let index = if identity.contains("gemini") {
            0
        } else if [
            "third_party",
            "third-party",
            "third party",
            "3p",
            "claude",
            "gpt",
        ]
        .iter()
        .any(|tag| identity.contains(tag))
        {
            1
        } else {
            return;
        };
        if !(0.0..=1.0).contains(&fraction) || resets_at.is_some_and(|t| t <= 0) {
            return;
        }
        let label = window.to_ascii_lowercase();
        let kind = if label.contains("week") {
            WindowKind::Weekly
        } else if ["five", "5h", "5-h", "5_h", "5 h"]
            .iter()
            .any(|tag| label.contains(tag))
        {
            WindowKind::FiveHour
        } else {
            WindowKind::Unspecified
        };
        if let Some((best, current)) = &mut self.windows[index]
            && (*best < fraction
                || (*best == fraction
                    && current.resets_at.unwrap_or(i64::MAX) <= resets_at.unwrap_or(i64::MAX)))
        {
            if *best == fraction
                && current.resets_at.is_none()
                && resets_at.is_none()
                && current.kind != kind
            {
                current.kind = WindowKind::Tied;
            }
            return;
        }
        self.windows[index] = Some((
            fraction,
            QuotaWindow {
                kind,
                remaining_percent: (fraction * 100.).round() as u8,
                resets_at,
            },
        ));
    }
    fn finish(self, captured: i64) -> Option<AntigravityQuota> {
        let [gemini, third] = self.windows.map(|w| {
            w.map(|(_, window)| GroupReading {
                window,
                stale: false,
                last_success: Some(captured),
            })
        });
        if gemini.is_none() && third.is_none() {
            return None;
        }
        Some(AntigravityQuota {
            gemini,
            claude_gpt: third,
        })
    }
}
fn reset(value: &Value) -> Option<i64> {
    Some(
        DateTime::parse_from_rfc3339(value.as_str()?)
            .ok()?
            .timestamp(),
    )
}
fn snapshot(path: &Path) -> Option<Source<AntigravityQuota>> {
    if !path.metadata().ok()?.is_file() {
        return None;
    }
    let file = File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > LIMIT {
        return None;
    }
    let captured = i64::try_from(
        metadata
            .modified()
            .ok()?
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_secs(),
    )
    .ok()?;
    let doc = json(file)?;
    let value = parse_report(&doc, captured)?;
    let age = metadata.modified().ok()?.elapsed().ok()?;
    Some(if age.as_secs() > 20 * 60 {
        Source::Stale(value)
    } else {
        Source::Current(value)
    })
}
pub fn observe_snapshot(
    path: &Path,
    previous: Option<&AntigravityQuota>,
) -> Source<AntigravityQuota> {
    Observer::default()
        .poll(Some(path), None, previous, chrono::Utc::now().timestamp())
        .0
}

fn parse_report(doc: &Value, captured: i64) -> Option<AntigravityQuota> {
    let mut groups = Groups::default();
    let mut add_bucket = |id: &str, window: &str, bucket: &Value| {
        if bucket["disabled"] == true {
            return;
        }
        let Some(f) = bucket["remaining_fraction"].as_f64() else {
            return;
        };
        let at = reset(&bucket["reset_time"]).filter(|t| *t > 0).or_else(|| {
            captured.checked_add(i64::try_from(bucket["reset_in_seconds"].as_u64()?).ok()?)
        });
        groups.add(id, window, f, at);
    };
    if let Some(buckets) = doc["quota"].as_object() {
        for (id, bucket) in buckets {
            add_bucket(id, id, bucket);
        }
    } else if doc["status"] == "SUCCESS" && doc["command"]["name"] == "usage" {
        for group in doc.pointer("/command/data/groups")?.as_array()? {
            for bucket in group["buckets"].as_array().into_iter().flatten() {
                if let Some(id) = bucket["id"].as_str() {
                    add_bucket(id, bucket["window"].as_str().unwrap_or(id), bucket);
                }
            }
        }
    } else {
        return None;
    }
    groups.finish(captured)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    MissingBinary,
    LaunchFailed,
    UnsupportedVersion,
    InvalidVersion,
    AuthenticationRequired,
    Timeout,
    Network,
    InvalidReport,
    MissingGroup,
    UnknownExit,
    SnapshotUnavailable,
}

impl Failure {
    fn launch(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::NotFound {
            Self::MissingBinary
        } else {
            Self::LaunchFailed
        }
    }

    fn retryable(self) -> bool {
        matches!(self, Self::Timeout | Self::Network | Self::MissingGroup)
    }
}

fn exit_reason(bytes: &[u8]) -> Failure {
    let msg = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    if [
        "unauthenticated",
        "not logged in",
        "authentication required",
        "invalid_grant",
        "please sign in",
    ]
    .iter()
    .any(|s| msg.contains(s))
    {
        Failure::AuthenticationRequired
    } else if ["deadline exceeded", "timed out"]
        .iter()
        .any(|s| msg.contains(s))
    {
        Failure::Timeout
    } else if [
        "network is unreachable",
        "connection refused",
        "connection reset",
        "no such host",
    ]
    .iter()
    .any(|s| msg.contains(s))
    {
        Failure::Network
    } else {
        Failure::UnknownExit
    }
}

fn cli_output(program: &Path, args: &[&str], seconds: u64) -> Result<Vec<u8>, Failure> {
    use std::{
        os::{
            fd::{AsRawFd, OwnedFd},
            unix::{net::UnixStream, process::CommandExt},
        },
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let read = |mut stream: UnixStream, limit: usize| -> Result<Vec<u8>, Failure> {
        let mut data = Vec::new();
        let mut buf = [0; 8192];
        loop {
            let left = deadline
                .checked_duration_since(Instant::now())
                .filter(|t| !t.is_zero())
                .ok_or(Failure::Timeout)?;
            // On Darwin, setting SO_RCVTIMEO can fail after a fast child has already closed its socket.
            let mut fd = libc::pollfd {
                fd: stream.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut fd, 1, left.as_millis().max(1) as i32) };
            if ready == 0 {
                return Err(Failure::Timeout);
            }
            if ready < 0 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(Failure::UnknownExit);
            }
            let n = match stream.read(&mut buf) {
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                result => result.map_err(|_| Failure::UnknownExit)?,
            };
            if n == 0 {
                return Ok(data);
            }
            if data.len() + n > limit {
                return Err(Failure::InvalidReport);
            }
            data.extend_from_slice(&buf[..n]);
        }
    };
    let (out, w) = UnixStream::pair().map_err(|_| Failure::UnknownExit)?;
    let (err, e) = UnixStream::pair().map_err(|_| Failure::UnknownExit)?;
    let mut cmd = Command::new(program);
    if let Ok(proxy) = std::env::var("AI_QUOTA_AGY_PROXY") {
        for key in [
            "http_proxy",
            "HTTP_PROXY",
            "https_proxy",
            "HTTPS_PROXY",
            "all_proxy",
            "ALL_PROXY",
        ] {
            cmd.env(key, &proxy);
        }
    }
    let mut child = cmd
        .args(args)
        .current_dir(std::env::temp_dir())
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(OwnedFd::from(w))
        .stderr(OwnedFd::from(e))
        .spawn()
        .map_err(Failure::launch)?;
    drop(cmd); // Release the parent copies of the child stream endpoints.
    let mut reaped = false;
    let result = std::thread::scope(|scope| {
        let stderr = scope.spawn(|| read(err, 64 * 1024));
        let bytes = read(out, LIMIT as usize)?;
        let errors = stderr.join().map_err(|_| Failure::UnknownExit)??;
        loop {
            if let Some(status) = child.try_wait().map_err(|_| Failure::UnknownExit)? {
                reaped = true;
                return if status.success() {
                    Ok(bytes)
                } else {
                    Err(exit_reason(if errors.is_empty() {
                        &bytes
                    } else {
                        &errors
                    }))
                };
            }
            if Instant::now() >= deadline {
                return Err(Failure::Timeout);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    });
    if !reaped {
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        let _ = child.wait();
    }
    result
}

#[derive(Default)]
pub struct Observer {
    verified: Option<(u64, u64, std::time::SystemTime)>,
    retries: usize,
}

impl Observer {
    fn usage(&mut self, path: &Path, now: i64) -> Result<AntigravityQuota, Failure> {
        use std::os::unix::fs::MetadataExt;
        let meta = path.metadata().map_err(Failure::launch)?;
        let identity = (
            meta.dev(),
            meta.ino(),
            meta.modified().map_err(|_| Failure::UnknownExit)?,
        );
        if self.verified != Some(identity) {
            let out = cli_output(path, &["--version"], 5)?;
            let parts: Vec<u32> = std::str::from_utf8(&out)
                .ok()
                .and_then(|s| {
                    s.trim()
                        .split('.')
                        .map(str::parse)
                        .collect::<Result<_, _>>()
                        .ok()
                })
                .ok_or(Failure::InvalidVersion)?;
            if parts.len() != 3 {
                return Err(Failure::InvalidVersion);
            }
            if parts.as_slice() < [1, 1, 11].as_slice() {
                return Err(Failure::UnsupportedVersion);
            }
            self.verified = Some(identity);
        }
        let out = cli_output(
            path,
            &[
                "-p",
                "/usage",
                "--output-format",
                "json",
                "--print-timeout",
                "20s",
            ],
            25,
        )?;
        let doc: Value = serde_json::from_slice(&out).map_err(|_| Failure::InvalidReport)?;
        if doc["status"] != "SUCCESS" {
            return Err(if doc["status"].is_string() {
                exit_reason(&out)
            } else {
                Failure::InvalidReport
            });
        }
        if doc["command"]["name"] != "usage" {
            return Err(Failure::InvalidReport);
        }
        parse_report(&doc, now).ok_or(Failure::InvalidReport)
    }

    pub fn poll(
        &mut self,
        path: Option<&Path>,
        program: Option<&Path>,
        previous: Option<&AntigravityQuota>,
        now: i64,
    ) -> (Source<AntigravityQuota>, [Option<Failure>; 2], i64) {
        let snap = path.and_then(snapshot);
        let fresh = matches!(snap, Some(Source::Current(_)));
        let snapshot = snap.as_ref().and_then(Source::value);
        let complete =
            fresh && snapshot.is_some_and(|q| q.gemini.is_some() && q.claude_gpt.is_some());
        let live = if complete {
            None
        } else {
            program.map(|p| self.usage(p, now))
        };
        let failure = match &live {
            Some(Err(e)) => *e,
            Some(Ok(_)) => Failure::MissingGroup,
            _ => Failure::SnapshotUnavailable,
        };
        let result = live.and_then(Result::ok);
        let select =
            |s: Option<&GroupReading>, l: Option<&GroupReading>, p: Option<&GroupReading>| {
                if fresh && let Some(v) = s {
                    return (Some(v.clone()), None);
                }
                if let Some(v) = l {
                    return (Some(v.clone()), None);
                }
                let stale = p
                    .into_iter()
                    .chain(s)
                    .max_by_key(|v| v.last_success)
                    .cloned()
                    .map(|mut v| {
                        v.stale = true;
                        v
                    });
                (stale, Some(failure))
            };
        let (gemini, g) = select(
            snapshot.and_then(|q| q.gemini.as_ref()),
            result.as_ref().and_then(|q| q.gemini.as_ref()),
            previous.and_then(|q| q.gemini.as_ref()),
        );
        let (claude_gpt, c) = select(
            snapshot.and_then(|q| q.claude_gpt.as_ref()),
            result.as_ref().and_then(|q| q.claude_gpt.as_ref()),
            previous.and_then(|q| q.claude_gpt.as_ref()),
        );
        let delay = if [g, c].into_iter().flatten().any(Failure::retryable) {
            let d = [15, 60, 300][self.retries];
            self.retries = (self.retries + 1).min(2);
            d
        } else {
            self.retries = 0;
            600
        };
        let absent = gemini.is_none() && claude_gpt.is_none();
        let quota = AntigravityQuota { gemini, claude_gpt };
        let source = if absent {
            Source::Unavailable
        } else if g.is_some() && c.is_some() {
            Source::Stale(quota)
        } else {
            Source::Current(quota)
        };
        (source, [g, c], delay)
    }
}
pub fn cli_quota(program: &Path) -> Option<AntigravityQuota> {
    Observer::default()
        .usage(program, chrono::Utc::now().timestamp())
        .ok()
}

pub fn cli_path() -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(p) = std::env::var_os("AI_QUOTA_AGY_BIN") {
        return std::path::absolute(p).ok();
    }
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|d| d.join("agy"))
        .find(|p| {
            p.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
        .and_then(|p| std::path::absolute(p).ok())
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/bin/agy")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stalled_process_hits_wall_clock_timeout() {
        let started = std::time::Instant::now();
        assert_eq!(
            cli_output(Path::new("/bin/sh"), &["-c", "sleep 30"], 1),
            Err(Failure::Timeout)
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }
}
