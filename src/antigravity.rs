use crate::model::{AntigravityQuota, QuotaWindow, Source, WindowKind};
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
    fn finish(self) -> Option<AntigravityQuota> {
        let [gemini, third] = self.windows;
        Some(AntigravityQuota {
            gemini: gemini?.1,
            claude_gpt: third?.1,
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
    observe(Some(path), previous).0
}

#[derive(Clone, Copy)]
pub enum SnapshotFailure {
    Expired,
    MissingOrInvalid,
}

pub fn observe(
    path: Option<&Path>,
    previous: Option<&AntigravityQuota>,
) -> (Source<AntigravityQuota>, Option<SnapshotFailure>) {
    match path.and_then(snapshot) {
        Some(source @ Source::Current(_)) => (source, None),
        Some(source) => (source, Some(SnapshotFailure::Expired)),
        None => (
            previous.cloned().map(Source::Stale).unwrap_or_default(),
            Some(SnapshotFailure::MissingOrInvalid),
        ),
    }
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
            for bucket in group["buckets"].as_array()? {
                add_bucket(bucket["id"].as_str()?, bucket["window"].as_str()?, bucket);
            }
        }
    } else {
        return None;
    }
    groups.finish()
}

fn cli_output(program: &Path, args: &[&str], seconds: u64) -> Option<Vec<u8>> {
    use std::{
        os::{
            fd::OwnedFd,
            unix::{net::UnixStream, process::CommandExt},
        },
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let (mut stream, writer) = UnixStream::pair().ok()?;
    let mut child = Command::new(program)
        .args(args)
        .current_dir(std::env::temp_dir())
        .process_group(0)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(OwnedFd::from(writer))
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let result = (|| {
        let mut bytes = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            stream
                .set_read_timeout(Some(deadline.checked_duration_since(Instant::now())?))
                .ok()?;
            let len = stream.read(&mut buf).ok()?;
            if len == 0 {
                break;
            }
            if bytes.len() + len > LIMIT as usize {
                return None;
            }
            bytes.extend_from_slice(&buf[..len]);
        }
        while Instant::now() < deadline {
            if let Some(status) = child.try_wait().ok()? {
                return status.success().then_some(bytes);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        None
    })();
    if result.is_none() && child.try_wait().ok().flatten().is_none() {
        unsafe {
            libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
        let _ = child.wait();
    }
    result
}

pub fn cli_quota(program: &Path) -> Option<AntigravityQuota> {
    // Older print-mode clients can interpret /usage as an ordinary model prompt.
    let version = String::from_utf8(cli_output(program, &["--version"], 3)?).ok()?;
    let parts: Vec<u32> = version
        .trim()
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if parts.len() != 3 || parts.as_slice() < [1, 1, 11].as_slice() {
        return None;
    }
    let output = cli_output(
        program,
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
    let doc = json(output.as_slice())?;
    if doc["command"]["name"] != "usage" || doc["status"] != "SUCCESS" {
        return None;
    }
    parse_report(&doc, chrono::Utc::now().timestamp())
}
