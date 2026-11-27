use ai_quota_zectrix_dashboard::{codex::CodexObserver, model::Source};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
use tempfile::TempDir;

struct Fake {
    dir: TempDir,
    program: PathBuf,
}
impl Fake {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("codex");
        fs::write(
            &program,
            r#"#!/usr/bin/env python3
import json, pathlib, subprocess, sys
root = pathlib.Path(__file__).parent
assert sys.argv[1:] == ['app-server','--stdio']
with (root/'starts').open('a') as f: print('start',file=f)
if (root/'spawn-helper').exists():
    helper = subprocess.Popen(['sleep','300'], stdout=subprocess.DEVNULL)
    (root/'helper').write_text(str(helper.pid))
initialized = False
for line in sys.stdin:
    msg = json.loads(line)
    method = msg['method']
    if method == 'initialize':
        result = {}
    elif method == 'initialized':
        initialized = True
        continue
    else:
        assert initialized and method == 'account/rateLimits/read'
        result = json.loads((root/'response').read_text())
        if result == 'close': sys.exit()
        if result == 'denied':
            print(json.dumps({'id':msg['id'],'error':{'code':-1,'message':'private'}}),flush=True)
            continue
    print(json.dumps({'id':msg['id'],'result':result}),flush=True)
(root/'closed').touch()
"#,
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        let fake = Self { dir, program };
        fake.respond(&normal());
        fake
    }
    fn respond(&self, value: &Value) {
        fs::write(self.dir.path().join("response"), value.to_string()).unwrap();
    }
    fn starts(&self) -> usize {
        fs::read_to_string(self.dir.path().join("starts"))
            .unwrap()
            .lines()
            .count()
    }
}
fn normal() -> Value {
    json!({"rateLimits":{"primary":{"usedPercent":99,"windowDurationMins":300,"resetsAt":100},
        "secondary":{"usedPercent":20,"windowDurationMins":10080,"resetsAt":200}},
        "rateLimitResetCredits":{"availableCount":3}})
}
#[test]
fn reads_weekly_quota_and_credits_on_one_owned_session() {
    let fake = Fake::new();
    let mut observer = CodexObserver::new(&fake.program);
    for _ in 0..2 {
        let Source::Current(quota) = observer.poll() else {
            panic!("missing quota")
        };
        assert_eq!(
            (
                quota.weekly.remaining_percent,
                quota.weekly.resets_at,
                quota.reset_credits
            ),
            (80, Some(200), Some(3))
        );
    }
    assert_eq!(fake.starts(), 1);
}

#[test]
fn invalid_windows_or_credit_counts_preserve_previous_evidence_until_a_valid_read() {
    let fake = Fake::new();
    let mut observer = CodexObserver::new(&fake.program);
    let Source::Current(known) = observer.poll() else {
        panic!("missing first reading")
    };
    for (path, bad) in [
        ("/rateLimits/primary/usedPercent", 101),
        ("/rateLimits/secondary/usedPercent", -1),
        ("/rateLimits/primary/windowDurationMins", 0),
        ("/rateLimits/secondary/windowDurationMins", -10),
        ("/rateLimitResetCredits/availableCount", -1),
    ] {
        let mut response = normal();
        *response.pointer_mut(path).unwrap() = json!(bad);
        fake.respond(&response);
        assert_eq!(observer.poll(), Source::Stale(known.clone()));
    }
    let mut response = normal();
    response["rateLimits"]["secondary"] = Value::Null;
    fake.respond(&response);
    assert_eq!(observer.poll(), Source::Unavailable);
    fake.respond(&Value::Null);
    assert_eq!(observer.poll(), Source::Unavailable);
    response["rateLimits"]["primary"]["windowDurationMins"] = json!(10080);
    response["rateLimitResetCredits"] = Value::Null;
    fake.respond(&response);
    let Source::Current(q) = observer.poll() else {
        panic!("missing weekly-only reading")
    };
    assert_eq!((q.weekly.remaining_percent, q.reset_credits), (1, None));
    assert_eq!(fake.starts(), 1);
}

#[test]
fn closed_transport_keeps_cache_and_backs_off_until_recovery() {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
        time::Duration,
    };
    let fake = Fake::new();
    let seconds = Arc::new(AtomicU64::new(0));
    let clock = seconds.clone();
    let mut observer = CodexObserver::with_clock(&fake.program, move || {
        Duration::from_secs(clock.load(Ordering::Relaxed))
    });
    let Source::Current(known) = observer.poll() else {
        panic!("missing first reading")
    };
    fake.respond(&json!("close"));
    let stale = Source::Stale(known.clone());
    assert_eq!(observer.poll(), stale);
    let mut elapsed = 0;
    for (i, delay) in [1, 2, 4, 8, 16, 32, 60, 60].into_iter().enumerate() {
        seconds.store(elapsed + delay - 1, Ordering::Relaxed);
        assert_eq!(observer.poll(), stale);
        assert_eq!(fake.starts(), i + 1);
        elapsed += delay;
        seconds.store(elapsed, Ordering::Relaxed);
        assert_eq!(observer.poll(), stale);
        assert_eq!(fake.starts(), i + 2);
    }
    fake.respond(&normal());
    seconds.store(elapsed + 60, Ordering::Relaxed);
    assert_eq!(observer.poll(), Source::Current(known.clone()));
    fake.respond(&json!("close"));
    assert_eq!(observer.poll(), stale);
    fake.respond(&normal());
    seconds.store(elapsed + 61, Ordering::Relaxed);
    assert_eq!(observer.poll(), Source::Current(known));
}

#[test]
fn live_preview_writes_a_weekly_quota_frame() {
    use std::process::Command;
    let fake = Fake::new();
    let path = fake.dir.path().join("live.png");
    let run = || {
        assert!(
            Command::new(env!("CARGO_BIN_EXE_ai-quota-zectrix-dashboard"))
                .arg("live-preview")
                .env("AI_QUOTA_DATA_DIR", fake.dir.path())
                .env("AI_QUOTA_AGY_BIN", fake.dir.path().join("missing-agy"))
                .env(
                    "AI_QUOTA_STATUSLINE_PATH",
                    fake.dir.path().join("no-snapshot")
                )
                .env("HTTPS_PROXY", "http://127.0.0.1:1")
                .env("NO_PROXY", "")
                .arg("--codex-program")
                .arg(&fake.program)
                .arg("--output")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        let img = image::open(&path).unwrap().to_luma8();
        assert_eq!(img.dimensions(), (400, 300));
        // Restrict comparison to the Codex dial, independent of wall-clock footer.
        image::imageops::crop_imm(&img, 0, 0, 400, 115).to_image()
    };
    let first = run();
    let mut response = normal();
    response["rateLimits"]["primary"]["usedPercent"] = json!(2);
    fake.respond(&response);
    assert_eq!(first, run());
    response["rateLimits"]["secondary"]["usedPercent"] = json!(81);
    fake.respond(&response);
    assert_ne!(first, run());
}

#[test]
fn owned_group_helpers_exit_after_drop_and_transport_failure() {
    for close in [false, true] {
        let fake = Fake::new();
        fs::write(fake.dir.path().join("spawn-helper"), "").unwrap();
        let mut observer = CodexObserver::new(&fake.program);
        assert!(matches!(observer.poll(), Source::Current(_)));
        let pid: i32 = fs::read_to_string(fake.dir.path().join("helper"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
        if close {
            fake.respond(&json!("close"));
            assert!(matches!(observer.poll(), Source::Stale(_)));
        } else {
            drop(observer);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while unsafe { libc::kill(pid, 0) } == 0 {
            assert!(std::time::Instant::now() < deadline, "helper survived");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

#[test]
fn restored_cache_survives_a_refused_read_without_restarting_a_healthy_session() {
    use ai_quota_zectrix_dashboard::codex::CodexFailure;
    let fake = Fake::new();
    let Source::Current(known) = CodexObserver::new(&fake.program).poll() else {
        panic!("missing quota")
    };
    fake.respond(&json!("denied"));
    let mut observer = CodexObserver::new(&fake.program);
    observer.restore_cache(known.clone());
    assert_eq!(observer.poll(), Source::Stale(known.clone()));
    assert_eq!(observer.last_failure(), Some(CodexFailure::Rejected));
    fake.respond(&normal());
    assert_eq!(observer.poll(), Source::Current(known));
    assert_eq!(observer.last_failure(), None);
    assert_eq!(fake.starts(), 2);
}

#[test]
fn healthy_server_can_finish_shutdown_on_stdin_eof() {
    let fake = Fake::new();
    assert!(matches!(
        CodexObserver::new(&fake.program).poll(),
        Source::Current(_)
    ));
    assert!(fake.dir.path().join("closed").exists());
}

#[test]
fn optional_short_window_does_not_hide_weekly_quota() {
    let fake = Fake::new();
    let mut observer = CodexObserver::new(&fake.program);
    for primary in [
        Value::Null,
        json!({"windowDurationMins":null,"resetsAt":null,"usedPercent":0}),
    ] {
        let mut response = normal();
        response["rateLimits"]["primary"] = primary;
        fake.respond(&response);
        assert!(matches!(observer.poll(), Source::Current(_)));
    }
}
