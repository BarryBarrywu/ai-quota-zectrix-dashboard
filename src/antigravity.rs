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
    fn add(&mut self, identity: &str, window: &str, fraction: f64, resets_at: i64) {
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
        if !(0.0..=1.0).contains(&fraction) || resets_at <= 0 {
            return;
        }
        if let Some((best, current)) = &self.windows[index]
            && (*best < fraction || (*best == fraction && current.resets_at <= resets_at))
        {
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
    let mut groups = Groups::default();
    for (id, bucket) in doc.get("quota")?.as_object()? {
        if bucket["disabled"] == true {
            continue;
        }
        let at = reset(&bucket["reset_time"]).or_else(|| {
            captured.checked_add(i64::try_from(bucket["reset_in_seconds"].as_u64()?).ok()?)
        });
        if let (Some(f), Some(at)) = (bucket["remaining_fraction"].as_f64(), at) {
            groups.add(id, id, f, at);
        }
    }
    let value = groups.finish()?;
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
