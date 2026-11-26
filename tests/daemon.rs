use std::process::Command;
const BIN: &str = env!("CARGO_BIN_EXE_ai-quota-zectrix-dashboard");

#[test]
fn diagnostics_without_observations_reports_every_source() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(BIN)
        .arg("diagnostics")
        .env("AI_QUOTA_DATA_DIR", dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    for source in [
        "codex_quota",
        "antigravity_quota",
        "codex_radar",
        "antigravity_radar",
        "publisher",
    ] {
        assert_eq!(report[source]["status"], "unavailable");
    }
}

#[test]
fn run_stops_on_sigterm_and_exposes_independent_failures() {
    use std::{
        thread,
        time::{Duration, Instant},
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("snapshot.json"),
        include_bytes!("data/agy-statusline.json"),
    )
    .unwrap();
    let mut daemon = Command::new(BIN)
        .args(["run", "--codex-program", "/nonexistent/codex"])
        .env("AI_QUOTA_DATA_DIR", dir.path())
        .env("AI_QUOTA_STATUSLINE_PATH", dir.path().join("snapshot.json"))
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("NO_PROXY", "")
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        assert!(daemon.try_wait().unwrap().is_none(), "daemon exited");
        if dir.path().join("diagnostics.json").exists() {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(30));
    }
    let out = Command::new(BIN)
        .arg("diagnostics")
        .env("AI_QUOTA_DATA_DIR", dir.path())
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["codex_quota"]["status"], "unavailable");
    assert_eq!(report["antigravity_quota"]["status"], "current");
    assert_eq!(report["publisher"]["reason"], "not_configured");
    unsafe {
        libc::kill(daemon.id() as i32, libc::SIGTERM);
    }
    assert!(daemon.wait().unwrap().success());
    let after = Command::new(BIN)
        .arg("diagnostics")
        .env("AI_QUOTA_DATA_DIR", dir.path())
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&after.stdout).unwrap();
    assert_eq!(report["antigravity_quota"]["status"], "stale");
}

#[test]
fn launch_agent_plist_preserves_paths_and_passes_only_configured_environment() {
    let dir = tempfile::tempdir().unwrap();
    let snapshot = dir.path().join("quota & reset.json");
    let out = Command::new(BIN)
        .args([
            "launch-agent",
            "plist",
            "--codex-program",
            "/Applications/Codex.app/Contents/Resources/codex",
        ])
        .env("AI_QUOTA_DATA_DIR", dir.path())
        .env("AI_QUOTA_STATUSLINE_PATH", &snapshot)
        .env("EXAMPLE_SECRET", "never-in-plist")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed = plist::Value::from_reader_xml(&out.stdout[..]).unwrap();
    let plist = parsed.as_dictionary().unwrap();
    let args = plist["ProgramArguments"].as_array().unwrap();
    assert_eq!(args[1].as_string(), Some("run"));
    assert_eq!(
        args[3].as_string(),
        Some("/Applications/Codex.app/Contents/Resources/codex")
    );
    let env = plist["EnvironmentVariables"].as_dictionary().unwrap();
    assert_eq!(
        env["AI_QUOTA_STATUSLINE_PATH"].as_string(),
        snapshot.to_str()
    );
    assert!(
        !String::from_utf8(out.stdout)
            .unwrap()
            .contains("never-in-plist")
    );
}

// The child uses the production scheduler and signal handling. Only the clock,
// loopback endpoints, credential and Codex executable differ from installation.
#[test]
#[ignore = "spawned by companion_process_with_local_sources"]
fn fixture_companion() {
    use ai_quota_zectrix_dashboard::{
        daemon::{Observers, run},
        publisher::Cloud,
        radar::{Provider, RadarObserver},
    };
    use chrono::{Local, TimeZone};
    use std::{
        env,
        path::PathBuf,
        time::{Duration, Instant},
    };
    let dir = PathBuf::from(env::var_os("FIXTURE_DIR").unwrap());
    let radars = [
        (Provider::Codex, "codex-radar.json", "CODEX_RADAR"),
        (Provider::Antigravity, "antigravity-radar.json", "AGY_RADAR"),
    ]
    .map(|(provider, file, address)| {
        RadarObserver::loopback(
            provider,
            dir.join(file),
            env::var(address).unwrap().parse().unwrap(),
        )
        .unwrap()
    });
    let observers = Observers::new(
        &dir,
        &dir.join("codex"),
        Some(dir.join("status.json")),
        radars,
    );
    let base: i64 = env::var("FIXTURE_EPOCH").unwrap().parse().unwrap();
    let start = std::cell::OnceCell::<Instant>::new();
    let scale = 180.;
    run(
        observers,
        || Cloud::new(&env::var("MOCK_CLOUD").unwrap(), "fixture-key").map(Some),
        || {
            Local
                .timestamp_opt(
                    base + (start.get().map_or(0., |t| t.elapsed().as_secs_f64()) * scale) as i64,
                    0,
                )
                .unwrap()
                .fixed_offset()
        },
        |s| {
            start.get_or_init(Instant::now);
            Duration::from_secs_f64(s as f64 / scale)
        },
    )
    .unwrap();
}

#[test]
fn companion_process_with_local_sources() {
    let dir = tempfile::tempdir().unwrap();
    let status = Command::new("python3")
        .args(["tests/support/companion.py"])
        .arg(std::env::current_exe().unwrap())
        .arg(BIN)
        .arg(dir.path())
        .status()
        .unwrap();
    assert!(status.success());
}
