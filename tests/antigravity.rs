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
    assert_eq!(quota.gemini.kind, WindowKind::FiveHour);
    assert_eq!(quota.gemini.remaining_percent, 33);
    assert_eq!(quota.claude_gpt.remaining_percent, 46);
    let mtime = fs::metadata(path)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap();
    assert_eq!(
        quota.claude_gpt.resets_at,
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
    assert_eq!(previous.claude_gpt.resets_at, Some(1_700_007_200));
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
    assert_eq!(q.gemini.kind, WindowKind::Weekly);
    assert_eq!(q.claude_gpt.kind, WindowKind::Tied);
    assert_eq!(q.claude_gpt.resets_at, None);
}
