use ai_quota_zectrix_dashboard::{
    model::{Radar, ResetKind, Scope, Source},
    radar::{Provider, RadarObserver},
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpListener},
    thread,
};

fn server(responses: Vec<String>) -> (SocketAddr, thread::JoinHandle<Vec<String>>) {
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = socket.local_addr().unwrap();
    let worker = thread::spawn(move || {
        responses
            .into_iter()
            .map(|response| {
                let (mut stream, _) = socket.accept().unwrap();
                let mut headers = String::new();
                let mut reader = BufReader::new(&stream);
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                    headers.push_str(&line);
                }
                let _ = stream.write_all(response.as_bytes());
                headers
            })
            .collect()
    });
    (address, worker)
}
fn ok(data: Value) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nETag: \"123\"\r\nConnection: close\r\n\r\n{}",
        json!({"ok":true, "data":data})
    )
}
fn signals() -> Value {
    json!({
        "nextScheduled":{"kind":"reset_scheduled","scheduleState":"pending","scheduleWindow":{
            "startAt":"1970-01-02T04:00:00Z","endAt":"1970-01-02T05:00:00Z"
        },"scope":{"modelScope":"third_party","modelIds":[]}},
        "latestGlobal":{"kind":"reset_completed","announcedAt":"1970-01-02T03:00:00Z","scope":{"modelScope":"gemini"}},
        "latestBanked":{"kind":"reset_completed","announcedAt":"1970-01-02T02:00:00Z"},
        "text":"private upstream prose"
    })
}
#[test]
fn pending_window_yields_to_latest_confirmation_until_24h() {
    let (addr, worker) = server(vec![ok(signals())]);
    let dir = tempfile::tempdir().unwrap();
    let mut observer =
        RadarObserver::loopback(Provider::Antigravity, dir.path().join("agy"), addr).unwrap();
    assert_eq!(
        observer.poll(100000),
        Source::Current(Radar::Scheduled {
            starts_at: 100800,
            ends_at: 104400,
            scope: Scope::ClaudeGpt
        })
    );
    assert_eq!(observer.next_expiry(100000), Some(104400));
    assert_eq!(
        observer.view(104400),
        Source::Current(Radar::Confirmed {
            kind: ResetKind::Global,
            announced_at: 97200,
            scope: Scope::Gemini
        })
    );
    assert_eq!(observer.view(183600), Source::Unavailable);
    assert!(worker.join().unwrap()[0].starts_with("GET /openapi/v1/status "));
}

#[test]
fn restart_restores_only_semantic_evidence_and_304_revalidates_it() {
    let (addr, worker) = server(vec![
        ok(signals()),
        "HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n".into(),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("radar").join("agy");
    let mut observer = RadarObserver::loopback(Provider::Antigravity, path.clone(), addr).unwrap();
    observer.poll(100000);
    let mut restarted = RadarObserver::loopback(Provider::Antigravity, path.clone(), addr).unwrap();
    assert!(matches!(
        restarted.view(100010),
        Source::Stale(Radar::Scheduled { .. })
    ));
    assert!(matches!(
        restarted.poll(100010),
        Source::Current(Radar::Scheduled { .. })
    ));
    let requests = worker.join().unwrap();
    assert!(
        requests[1]
            .to_lowercase()
            .contains("if-none-match: \"123\"")
    );
    assert!(
        !std::fs::read_to_string(path)
            .unwrap()
            .contains("upstream prose")
    );
}

#[test]
fn retry_after_blocks_early_requests_even_after_restart() {
    let (addr, worker) = server(vec![
        ok(signals()),
        "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 8000\r\nConnection: close\r\n\r\n".into(),
        ok(signals()),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agy");
    let mut observer = RadarObserver::loopback(Provider::Antigravity, path.clone(), addr).unwrap();
    observer.poll(100000);
    assert!(matches!(
        observer.poll(103600),
        Source::Stale(Radar::Scheduled { .. })
    ));
    assert_eq!(observer.next_poll_at(), 111600);
    let mut restarted = RadarObserver::loopback(Provider::Antigravity, path, addr).unwrap();
    assert!(matches!(
        restarted.poll(111599),
        Source::Stale(Radar::Confirmed { .. })
    ));
    assert!(matches!(
        restarted.poll(111600),
        Source::Current(Radar::Confirmed { .. })
    ));
    assert_eq!(worker.join().unwrap().len(), 3);
}

#[test]
fn failures_preserve_eligible_events_and_bound_untrusted_body() {
    let mut huge = signals();
    huge["text"] = Value::String("z".repeat(65536));
    let (addr, worker) = server(vec![
        ok(signals()),
        ok(huge),
        "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{secret".into(),
        "HTTP/1.1 500 Failed\r\nConnection: close\r\n\r\nsecret".into(),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let mut observer =
        RadarObserver::loopback(Provider::Antigravity, dir.path().join("agy"), addr).unwrap();
    observer.poll(100000);
    assert!(matches!(
        observer.poll(103600),
        Source::Stale(Radar::Scheduled { .. })
    ));
    assert_eq!(observer.last_error(), Some("response too large"));
    assert!(matches!(
        observer.poll(107200),
        Source::Stale(Radar::Confirmed { .. })
    ));
    assert_eq!(observer.last_error(), Some("invalid status"));
    assert_eq!(observer.poll(183600), Source::Unavailable);
    assert_eq!(observer.last_error(), Some("http error"));
    worker.join().unwrap();
}

#[test]
fn slow_stream_cannot_extend_five_second_total_deadline() {
    use std::time::{Duration, Instant};
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = socket.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let (mut stream, _) = socket.accept().unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
            .unwrap();
        for _ in 0..7 {
            if stream.write_all(b" ").is_err() {
                break;
            }
            thread::sleep(Duration::from_secs(1));
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let mut observer =
        RadarObserver::loopback(Provider::Codex, dir.path().join("codex"), addr).unwrap();
    let start = Instant::now();
    assert_eq!(observer.poll(100000), Source::Unavailable);
    assert!(start.elapsed() < Duration::from_secs(6));
    worker.join().unwrap();
}

#[test]
fn quiet_observation_expires_and_requires_revalidation_after_restart() {
    let quiet = json!({"nextScheduled":null,"latestGlobal":null,"latestBanked":null});
    let (addr, worker) = server(vec![
        ok(quiet),
        "HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n".into(),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("codex");
    let mut observer = RadarObserver::loopback(Provider::Codex, path.clone(), addr).unwrap();
    assert_eq!(observer.poll(100000), Source::Current(Radar::NoSchedule));
    assert_eq!(observer.next_expiry(100000), Some(107200));
    assert_eq!(observer.view(107200), Source::Unavailable);
    let mut restarted = RadarObserver::loopback(Provider::Codex, path, addr).unwrap();
    assert_eq!(restarted.view(100001), Source::Unavailable);
    assert_eq!(restarted.poll(100001), Source::Current(Radar::NoSchedule));
    worker.join().unwrap();
}

#[test]
fn malformed_event_fields_do_not_erase_last_confirmation() {
    let (addr, worker) = server(vec![
        ok(signals()),
        ok(json!({"nextScheduled":false, "latestGlobal":{}, "latestBanked":"secret"})),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let mut observer =
        RadarObserver::loopback(Provider::Codex, dir.path().join("codex"), addr).unwrap();
    observer.poll(100000);
    assert!(matches!(
        observer.poll(107200),
        Source::Stale(Radar::Confirmed { .. })
    ));
    assert_eq!(observer.last_error(), Some("invalid status"));
    worker.join().unwrap();
}
