use ai_quota_zectrix_dashboard::{
    antigravity::observe_snapshot,
    model::{Source, WindowKind},
};
use std::fs;
use std::time::UNIX_EPOCH;

#[test]
fn statusline_uses_constrained_window_and_capture_time() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path();
    fs::write(path, include_bytes!("data/agy-statusline.json")).unwrap();
    let Source::Current(quota) = observe_snapshot(path, None) else {
        panic!("missing groups")
    };
    assert_eq!(
        quota.gemini.as_ref().unwrap().window.kind,
        WindowKind::FiveHour
    );
    assert_eq!(quota.gemini.as_ref().unwrap().window.remaining_percent, 33);
    assert_eq!(
        quota.claude_gpt.as_ref().unwrap().window.remaining_percent,
        46
    );
    let mtime = fs::metadata(path)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap();
    assert_eq!(
        quota.claude_gpt.as_ref().unwrap().window.resets_at,
        Some(mtime.as_secs() as i64 + 7200)
    );
}

#[test]
fn stale_snapshot_keeps_its_capture_time_and_invalid_data_uses_previous() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    fs::write(tmp.path(), include_bytes!("data/agy-statusline.json")).unwrap();
    tmp.as_file()
        .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000))
        .unwrap();
    let Source::Stale(previous) = observe_snapshot(tmp.path(), None) else {
        panic!("expected stale snapshot")
    };
    assert_eq!(
        previous.claude_gpt.as_ref().unwrap().window.resets_at,
        Some(1_700_007_200)
    );
    fs::write(
        tmp.path(),
        br#"{"context_window":{"remaining_percentage":0}}"#,
    )
    .unwrap();
    assert_eq!(
        observe_snapshot(tmp.path(), Some(&previous)),
        Source::Stale(previous)
    );
    assert_eq!(observe_snapshot(tmp.path(), None), Source::Unavailable);
}

#[test]
fn cli_usage_report_handles_absent_reset_and_tied_full_windows() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    fs::write(tmp.path(), r#"{"status":"SUCCESS","command":{"name":"usage","data":{"groups":[
      {"buckets":[{"id":"gemini-weekly","window":"weekly","remaining_fraction":1,"reset_time":"2026-12-03T14:10:00Z"}]},
      {"buckets":[{"id":"3p-weekly","window":"weekly","remaining_fraction":1,"reset_time":"1970-01-01T00:00:00Z"},
                  {"id":"3p-5h","window":"5h","remaining_fraction":1,"reset_time":"1970-01-01T00:00:00Z"}]}
    ]}}}"#).unwrap();
    let Source::Current(q) = observe_snapshot(tmp.path(), None) else {
        panic!("missing CLI usage")
    };
    assert_eq!(q.gemini.as_ref().unwrap().window.kind, WindowKind::Weekly);
    assert_eq!(q.claude_gpt.as_ref().unwrap().window.kind, WindowKind::Tied);
    assert_eq!(q.claude_gpt.as_ref().unwrap().window.resets_at, None);
}

#[test]
fn live_cli_reads_a_structured_usage_command() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("agy");
    fs::write(&program, r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo 1.5.18; exit; fi
[ "$*" = "-p /usage --output-format json --print-timeout 20s" ] || exit 1
echo '{"status":"SUCCESS","command":{"name":"usage","data":{"groups":[{"buckets":[{"id":"gemini-weekly","window":"weekly","remaining_fraction":0.76},{"id":"3p-weekly","window":"weekly","remaining_fraction":1}]}]}}}'
"#).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    let (report, reasons, _) = ai_quota_zectrix_dashboard::antigravity::Observer::default().poll(
        None,
        Some(&program),
        None,
        1000,
    );
    let quota = report
        .value()
        .unwrap_or_else(|| panic!("CLI failed: {reasons:?}"));
    assert_eq!(
        (
            quota.gemini.as_ref().unwrap().window.remaining_percent,
            quota.claude_gpt.as_ref().unwrap().window.remaining_percent
        ),
        (76, 100)
    );
}

struct Script {
    dir: tempfile::TempDir,
    bin: std::path::PathBuf,
}
impl Script {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("agy");
        fs::write(
            &bin,
            r#"#!/bin/sh
cd "$(dirname "$0")"
if [ "$1" = "--version" ]; then echo v >> probes; cat version; exit; fi
echo usage >> calls
if test -f error; then cat error >&2; exit 1; fi
cat report
"#,
        )
        .unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(dir.path().join("version"), "1.5.18").unwrap();
        Self { dir, bin }
    }
    fn report(&self, buckets: serde_json::Value) {
        fs::write(self.dir.path().join("report"),serde_json::json!({"status":"SUCCESS","command":{"name":"usage","data":{"groups":[{"buckets":buckets}]}}}).to_string()).unwrap();
    }
}

#[test]
fn snapshot_and_cli_fill_groups_independently_and_keep_the_original_observation_times() {
    use ai_quota_zectrix_dashboard::antigravity::{Failure, Observer};
    use serde_json::json;
    let f = Script::new();
    let p = f.dir.path().join("snapshot");
    let mut observer = Observer::default();
    fs::write(&p, include_bytes!("data/agy-statusline.json")).unwrap();
    let (full, errors, _) = observer.poll(Some(&p), Some(&f.bin), None, 1000);
    assert_eq!(errors, [None, None]);
    assert!(!f.dir.path().join("probes").exists());
    fs::write(
        &p,
        r#"{"quota":{"gemini-weekly":{"remaining_fraction":0.91}}}"#,
    )
    .unwrap();
    f.report(json!([{"id":"3p-weekly","window":"weekly","remaining_fraction":0.77}]));
    let (both, errors, _) = observer.poll(Some(&p), Some(&f.bin), full.value(), 2000);
    assert_eq!(errors, [None, None]);
    let q = both.value().unwrap();
    assert_eq!(q.gemini.as_ref().unwrap().window.remaining_percent, 91);
    assert_eq!(q.claude_gpt.as_ref().unwrap().last_success, Some(2000));
    fs::File::open(&p)
        .unwrap()
        .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(20))
        .unwrap();
    f.report(json!([{"id":"gemini-weekly","window":"weekly","remaining_fraction":0.85}]));
    let (mixed, errors, delay) = observer.poll(Some(&p), Some(&f.bin), Some(q), 3000);
    assert_eq!(errors, [None, Some(Failure::MissingGroup)]);
    assert_eq!(delay, 15);
    let m = mixed.value().unwrap();
    let g = m.gemini.as_ref().unwrap();
    let c = m.claude_gpt.as_ref().unwrap();
    assert!(!g.stale);
    assert_eq!(g.window.remaining_percent, 85);
    assert!(c.stale);
    assert_eq!(
        (c.window.remaining_percent, c.last_success),
        (77, Some(2000))
    );
    let snapshotless = observer.poll(None, Some(&f.bin), None, 3015).0;
    assert!(snapshotless.value().unwrap().claude_gpt.is_none());
    let mut d: ai_quota_zectrix_dashboard::model::Dashboard =
        serde_json::from_str(include_str!("../fixtures/normal.json")).unwrap();
    d.antigravity = mixed;
    assert!(
        ai_quota_zectrix_dashboard::render(&d)
            .unwrap()
            .visible_text
            .iter()
            .any(|v| v == "旧")
    );
    d.antigravity = snapshotless;
    assert!(
        ai_quota_zectrix_dashboard::render(&d)
            .unwrap()
            .visible_text
            .iter()
            .any(|v| v == "暂不可用")
    );
    assert_eq!(
        fs::read_to_string(f.dir.path().join("probes")).unwrap(),
        "v\n"
    );
}

#[test]
fn transient_errors_back_off_and_permanent_errors_are_sanitized() {
    use ai_quota_zectrix_dashboard::antigravity::{Failure, Observer};
    let f = Script::new();
    let mut observer = Observer::default();
    let err = f.dir.path().join("error");
    fs::write(&err, "context deadline exceeded for private-account").unwrap();
    for delay in [15, 60, 300, 300] {
        let (_, errors, next) = observer.poll(None, Some(&f.bin), None, 1000);
        assert_eq!((errors, next), ([Some(Failure::Timeout); 2], delay));
    }
    for (text, reason) in [
        ("not logged in: secret", Failure::AuthenticationRequired),
        ("unexpected private message", Failure::UnknownExit),
    ] {
        fs::write(&err, text).unwrap();
        assert_eq!(
            observer.poll(None, Some(&f.bin), None, 1000).1,
            [Some(reason); 2]
        );
        assert_eq!(observer.poll(None, Some(&f.bin), None, 1000).2, 600);
    }
    fs::remove_file(err).unwrap();
    fs::write(f.dir.path().join("report"), "not json").unwrap();
    assert_eq!(
        observer.poll(None, Some(&f.bin), None, 1000).1,
        [Some(Failure::InvalidReport); 2]
    );
    let replacement = f.dir.path().join("new");
    fs::copy(&f.bin, &replacement).unwrap();
    fs::rename(replacement, &f.bin).unwrap();
    fs::write(f.dir.path().join("version"), "1.0.0").unwrap();
    assert_eq!(
        observer.poll(None, Some(&f.bin), None, 1000).1,
        [Some(Failure::UnsupportedVersion); 2]
    );
    assert_eq!(
        fs::read_to_string(f.dir.path().join("probes")).unwrap(),
        "v\nv\n"
    );
    fs::remove_file(&f.bin).unwrap();
    assert_eq!(
        observer.poll(None, Some(&f.bin), None, 1000).1,
        [Some(Failure::MissingBinary); 2]
    );
}

#[test]
fn cli_accepts_prefixed_and_prerelease_semver_strings() {
    use ai_quota_zectrix_dashboard::antigravity::Observer;
    use serde_json::json;
    let f = Script::new();
    f.report(json!([
        {"id":"gemini-weekly","window":"weekly","remaining_fraction":0.80},
        {"id":"3p-5h","window":"5h","remaining_fraction":0.60}
    ]));
    fs::write(f.dir.path().join("version"), "agy v1.6.0-beta.2+abc123\n").unwrap();
    let mut observer = Observer::default();
    let (source, errors, delay) = observer.poll(None, Some(&f.bin), None, 1000);
    assert_eq!(errors, [None, None]);
    assert_eq!(delay, 600);
    let q = source.value().unwrap();
    assert_eq!(q.gemini.as_ref().unwrap().window.remaining_percent, 80);
    assert_eq!(q.claude_gpt.as_ref().unwrap().window.remaining_percent, 60);
}
