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
    assert_eq!(quota.claude_gpt.resets_at, mtime.as_secs() as i64 + 7200);
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
    assert_eq!(previous.claude_gpt.resets_at, 1_700_007_200);
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
