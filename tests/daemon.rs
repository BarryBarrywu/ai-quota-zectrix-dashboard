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

#[test]
fn installation_discovers_agy_in_path_and_only_persists_an_explicit_safe_proxy() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("agy");
    std::fs::write(&program, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut cmd = Command::new(BIN);
    cmd.args(["launch-agent", "plist", "--codex-program", "/usr/bin/true"])
        .env_remove("AI_QUOTA_AGY_BIN")
        .env("PATH", dir.path())
        .env("HTTP_PROXY", "http://user:private@host")
        .env("AI_QUOTA_AGY_PROXY", "http://127.0.0.1:8765");
    let out = cmd.output().unwrap();
    assert!(out.status.success());
    let plist = plist::Value::from_reader_xml(&out.stdout[..]).unwrap();
    let vars = plist.as_dictionary().unwrap()["EnvironmentVariables"]
        .as_dictionary()
        .unwrap();
    assert_eq!(vars["AI_QUOTA_AGY_BIN"].as_string(), program.to_str());
    assert_eq!(
        vars["AI_QUOTA_AGY_PROXY"].as_string(),
        Some("http://127.0.0.1:8765")
    );
    assert!(!vars.contains_key("HTTP_PROXY"));
    let rejected = cmd
        .env("AI_QUOTA_AGY_PROXY", "http://user:private@host")
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(!String::from_utf8_lossy(&rejected.stderr).contains("private"));
}

#[test]
fn restarted_daemon_preserves_the_other_groups_last_success() {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        time::{Duration, Instant},
    };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let agy = root.join("agy");
    fs::write(&agy,r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo 1.5.18; exit; fi
echo '{"status":"SUCCESS","command":{"name":"usage","data":{"groups":[{"buckets":[{"id":"gemini-weekly","window":"weekly","remaining_fraction":0.91}]}]}}}'
"#).unwrap();
    fs::set_permissions(&agy, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(root.join("antigravity-quota.json"),r#"{"gemini":{"kind":"weekly","remaining_percent":3,"resets_at":null},"claude_gpt":{"kind":"weekly","remaining_percent":72,"resets_at":null,"last_success":4567}}"#).unwrap();
    for _ in 0..2 {
        let mut child = Command::new(BIN)
            .args(["run", "--codex-program", "/nonexistent/codex"])
            .env("AI_QUOTA_DATA_DIR", root)
            .env("AI_QUOTA_AGY_BIN", &agy)
            .env("AI_QUOTA_STATUSLINE_PATH", root.join("missing"))
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .env("NO_PROXY", "")
            .spawn()
            .unwrap();
        let until = Instant::now() + Duration::from_secs(12);
        let path = root.join("diagnostics.json");
        while !path.exists() {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(20));
        }
        let report: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        unsafe {
            libc::kill(child.id() as i32, libc::SIGTERM);
        }
        assert!(child.wait().unwrap().success());
        assert_eq!(report["gemini"]["status"], "current");
        assert!(report["gemini"]["last_success"].as_i64().unwrap() > 4567);
        assert_eq!(report["claude_gpt"]["status"], "stale");
        assert_eq!(report["claude_gpt"]["reason"], "missing_group");
        assert_eq!(report["claude_gpt"]["last_success"], 4567);
        let cached: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("antigravity-quota.json")).unwrap())
                .unwrap();
        assert_eq!(cached["claude_gpt"]["remaining_percent"], 72);
        fs::remove_file(path).unwrap();
    }
}
