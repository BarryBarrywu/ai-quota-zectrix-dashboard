use ai_quota_zectrix_dashboard::publisher::Cloud;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;

fn server(replies: Vec<&'static str>) -> (String, thread::JoinHandle<Vec<Vec<u8>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut requests = vec![];
        for reply in replies {
            let (mut socket, _) = listener.accept().unwrap();
            let mut r = BufReader::new(&socket);
            let mut request = vec![];
            let mut size = 0;
            loop {
                let mut line = String::new();
                assert!(r.read_line(&mut line).unwrap() > 0);
                request.extend(line.as_bytes());
                if line == "\r\n" {
                    break;
                }
                if let Some(n) = line.to_lowercase().strip_prefix("content-length: ") {
                    size = n.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; size];
            r.read_exact(&mut body).unwrap();
            request.extend(body);
            requests.push(request);
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            )
            .unwrap();
        }
        requests
    });
    (base, handle)
}

const DEVICES: &str = r#"{"code":0,"data":[{"deviceId":"legacy","board":"unsupported","alias":"Other"},{"deviceId":"note","board":"zectrix-s3-epaper-4.2","alias":"Desk"},{"deviceId":"compact","board":"bread-compact-wifi","alias":"Bedroom"},{"deviceId":"localized","board":"极趣 S3 墨水屏 4.2寸","alias":"Studio"}]}"#;

#[test]
fn discovery_lists_only_compatible_boards() {
    let (base, server) = server(vec![DEVICES]);
    let cloud = Cloud::new(&base, "fixture-key").unwrap();
    let devices = cloud.devices().unwrap();
    assert_eq!(
        devices.iter().map(|d| d.alias.as_str()).collect::<Vec<_>>(),
        ["Desk", "Bedroom", "Studio"]
    );
    let req = String::from_utf8(server.join().unwrap().remove(0))
        .unwrap()
        .to_lowercase();
    assert!(req.starts_with("get /open/v1/devices "));
    assert!(req.contains("x-api-key: fixture-key"));
}

use ai_quota_zectrix_dashboard::{
    model::Dashboard,
    publisher::{PublishOutcome, Settings, publish_changed},
};

#[test]
fn publisher_observes_visible_countdowns_and_respects_persisted_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let (base, server) = server(vec![r#"{"code":0}"#, r#"{"code":0}"#]);
    let cloud = Cloud::new(&base, "fixture-key").unwrap();
    let settings = Settings {
        device_id: "note".into(),
        page_id: 3,
    };
    let mut d: Dashboard = serde_json::from_str(include_str!("../fixtures/normal.json")).unwrap();
    // Keep the visible minute label stable for the first ten seconds.
    d.now += chrono::Duration::seconds(1);
    let start = d.now;
    let send = |d: &Dashboard| publish_changed(dir.path(), &cloud, &settings, d).unwrap();
    assert_eq!(send(&d), PublishOutcome::Published);
    d.now += chrono::Duration::seconds(10);
    d.screen_updated_at = Some(d.now);
    assert_eq!(send(&d), PublishOutcome::Unchanged);
    let mut changed = d.clone();
    changed.antigravity = Default::default();
    assert_eq!(
        send(&changed),
        PublishOutcome::Deferred {
            until: start.timestamp() + 60
        }
    );
    d.now = start + chrono::Duration::seconds(60);
    assert_eq!(send(&d), PublishOutcome::Unchanged);
    d.now = start + chrono::Duration::seconds(900);
    assert_eq!(send(&d), PublishOutcome::Published);
    let requests = server.join().unwrap();
    for req in requests {
        let payload = String::from_utf8_lossy(&req);
        assert!(payload.starts_with("POST /open/v1/devices/note/display/image "));
        assert!(payload.contains("name=\"pageId\"\r\n\r\n3"));
        assert!(payload.contains("name=\"dither\"\r\n\r\nfalse"));
        assert!(req.windows(8).any(|s| s == b"\x89PNG\r\n\x1a\n"));
    }
}

use ai_quota_zectrix_dashboard::{publisher::Result as PublishResult, setup::configure_with_clock};

fn configure(
    dir: &std::path::Path,
    cloud: &Cloud,
    dashboard: &Dashboard,
    input: &mut impl std::io::BufRead,
    output: &mut impl Write,
    save_key: impl FnOnce() -> PublishResult<()>,
) -> PublishResult<bool> {
    configure_with_clock(dir, cloud, dashboard, input, output, save_key, || {
        dashboard.now
    })
}
use std::fs;

#[test]
fn setup_requires_confirmation_after_generating_preview() {
    let dir = tempfile::tempdir().unwrap();
    let (base, server) = server(vec![DEVICES]);
    let cloud = Cloud::new(&base, "fixture-key").unwrap();
    let d = serde_json::from_str(include_str!("../fixtures/normal.json")).unwrap();
    let mut output = vec![];
    let preview = dir.path().join("pending-preview.png");
    let mut answers = &b"2\n4\n"[..];
    // At the confirmation prompt, inspect the file exactly as the operator would.
    let inspect = std::io::Cursor::new(b"n\n");
    struct Inspect<F, R>(F, R);
    impl<F: FnMut(), R: Read> Read for Inspect<F, R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            (self.0)();
            self.1.read(buf)
        }
    }
    let mut input = BufReader::new(answers.by_ref().chain(Inspect(
        || {
            assert!(fs::read(&preview).unwrap().starts_with(b"\x89PNG"));
            assert!(!dir.path().join("settings.json").exists());
        },
        inspect,
    )));
    assert!(
        !configure(dir.path(), &cloud, &d, &mut input, &mut output, || panic!(
            "saved declined key"
        ))
        .unwrap()
    );
    assert!(!dir.path().join("settings.json").exists());
    assert_eq!(server.join().unwrap().len(), 1);
    let output = String::from_utf8(output).unwrap();
    assert!(!output.contains("fixture-key"));
    assert!(!output.contains("Other"));
}

#[test]
fn approved_setup_uploads_the_reviewed_image_and_saves_only_the_destination() {
    let dir = tempfile::tempdir().unwrap();
    let (base, server) = server(vec![DEVICES, r#"{"code":0}"#]);
    let cloud = Cloud::new(&base, "fixture-key").unwrap();
    let d = serde_json::from_str(include_str!("../fixtures/normal.json")).unwrap();
    let mut saved = false;
    assert!(
        configure(
            dir.path(),
            &cloud,
            &d,
            &mut &b"2\n4\ny\n"[..],
            &mut vec![],
            || {
                saved = true;
                Ok(())
            }
        )
        .unwrap()
    );
    assert!(saved);
    let req = server.join().unwrap().pop().unwrap();
    assert!(req.starts_with(b"POST /open/v1/devices/compact/display/image "));
    let preview = fs::read(dir.path().join("pending-preview.png")).unwrap();
    assert!(req.windows(preview.len()).any(|s| s == preview));
    let settings: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("settings.json")).unwrap()).unwrap();
    assert_eq!(
        settings,
        serde_json::json!({"deviceId":"compact", "pageId":4})
    );
}

#[test]
fn rejected_upload_preserves_existing_settings_and_key_and_reserves_retry_interval() {
    let dir = tempfile::tempdir().unwrap();
    let previous = b"{ \"deviceId\": \"note\", \"pageId\": 5 }\n";
    fs::write(dir.path().join("settings.json"), previous).unwrap();
    let (base, server) = server(vec![DEVICES, r#"{"code":17}"#, DEVICES]);
    let cloud = Cloud::new(&base, "fixture-key").unwrap();
    let d = serde_json::from_str(include_str!("../fixtures/normal.json")).unwrap();
    for why in ["拒绝", "间隔"] {
        let error = configure(
            dir.path(),
            &cloud,
            &d,
            &mut &b"2\n3\ny\n"[..],
            &mut vec![],
            || panic!("overwrote a key on failed setup"),
        )
        .unwrap_err();
        assert!(error.to_string().contains(why));
        assert_eq!(
            fs::read(dir.path().join("settings.json")).unwrap(),
            previous
        );
    }
    assert_eq!(server.join().unwrap().len(), 3);
}

#[test]
fn setup_command_is_available_from_the_companion() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ai-quota-zectrix-dashboard"))
        .args(["setup", "--help"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_running_publisher_respects_a_newly_configured_page() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("settings.json"),
        br#"{"deviceId":"note","pageId":4}"#,
    )
    .unwrap();
    let stale = Settings {
        device_id: "note".into(),
        page_id: 1,
    };
    let (base, server) = server(vec![]);
    server.join().unwrap();
    let cloud = Cloud::new(&base, "fixture-key").unwrap();
    let d = serde_json::from_str(include_str!("../fixtures/normal.json")).unwrap();
    assert!(publish_changed(dir.path(), &cloud, &stale, &d).is_err());
    assert!(!dir.path().join("publisher.json").exists());
}

#[test]
fn elapsed_confirmation_requires_review_of_new_frame() {
    use ai_quota_zectrix_dashboard::{model::Dashboard, setup::configure_with_clock};
    let dir = tempfile::tempdir().unwrap();
    let (base, server) = server(vec![DEVICES]);
    let cloud = Cloud::new(&base, "fixture-key").unwrap();
    let dashboard: Dashboard =
        serde_json::from_str(include_str!("../fixtures/normal.json")).unwrap();
    let mut clock = [dashboard.now, dashboard.now + chrono::Duration::minutes(2)].into_iter();
    let mut output = Vec::new();
    assert!(
        !configure_with_clock(
            dir.path(),
            &cloud,
            &dashboard,
            &mut &b"1\n1\ny\nn\n"[..],
            &mut output,
            || panic!("declined"),
            || clock.next().unwrap()
        )
        .unwrap()
    );
    assert!(String::from_utf8(output).unwrap().contains("时间已更新"));
    assert_eq!(server.join().unwrap().len(), 1);
}
