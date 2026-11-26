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
fn observation(
    value: Option<AntigravityQuota>,
    previous: Option<&AntigravityQuota>,
) -> Source<AntigravityQuota> {
    value
        .map(Source::Current)
        .or_else(|| previous.cloned().map(Source::Stale))
        .unwrap_or_default()
}
pub fn observe_snapshot(
    path: &Path,
    previous: Option<&AntigravityQuota>,
) -> Source<AntigravityQuota> {
    snapshot(path).unwrap_or_else(|| observation(None, previous))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::{TcpListener, TcpStream},
        thread,
    };

    fn mock(responses: Vec<(&'static str, &'static str)>) -> Option<AntigravityQuota> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for (method, response) in responses {
                let (mut socket, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(&mut socket);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert_eq!(
                    line.trim(),
                    format!("POST /exa.language_server_pb.LanguageServerService/{method} HTTP/1.1")
                );
                let mut headers = String::new();
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    headers.push_str(&line.to_ascii_lowercase());
                }
                assert!(headers.contains("x-codeium-csrf-token: test"));
                let mut body = [0; 2];
                reader.read_exact(&mut body).unwrap();
                assert_eq!(&body, b"{}");
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                    response.len(),
                    response
                )
                .unwrap();
            }
        });
        let result = query(|| Some((TcpStream::connect(addr).ok()?, String::from("test"))));
        server.join().unwrap();
        result
    }
    #[test]
    fn legacy_fallback_supports_omitted_zero_and_unspecified_window() {
        let quota = mock(vec![("RetrieveUserQuotaSummary", "{}"), ("GetUserStatus", r#"{"userStatus":{"cascadeModelConfigData":{"clientModelConfigs":[{"label":"Gemini Pro","quotaInfo":{"resetTime":"2026-11-28T00:00:00Z"}},{"label":"Claude","quotaInfo":{"remainingFraction":0.23,"resetTime":"2026-11-28T02:00:00Z"}}]}}}"#)]).unwrap();
        assert_eq!(quota.gemini.remaining_percent, 0);
        assert_eq!(quota.claude_gpt.remaining_percent, 23);
        assert_eq!(quota.claude_gpt.kind, WindowKind::Unspecified);
    }
    #[test]
    fn grouped_summary_selects_by_fraction_before_rounding() {
        let quota = mock(vec![("RetrieveUserQuotaSummary", r#"{"response":{"groups":[{"displayName":"Gemini","buckets":[{"window":"QUOTA_WINDOW_WEEKLY","remainingFraction":0.684,"resetTime":"2026-11-27T00:00:00Z"},{"window":"QUOTA_WINDOW_FIVE_HOUR","remainingFraction":0.681,"resetTime":"2026-11-28T00:00:00Z"}]},{"displayName":"Third Party","buckets":[{"window":"WEEKLY","remainingFraction":0,"resetTime":"2026-11-28T00:00:00Z"}]}]}}"#)]).unwrap();
        assert_eq!(quota.gemini.kind, WindowKind::FiveHour);
        assert_eq!(quota.claude_gpt.remaining_percent, 0);
    }
}

fn query(
    mut connect: impl FnMut() -> Option<(std::net::TcpStream, String)>,
) -> Option<AntigravityQuota> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    for method in ["RetrieveUserQuotaSummary", "GetUserStatus"] {
        let (socket, token) = connect()?;
        let response = rt.block_on(async {
            tokio::time::timeout(
                std::time::Duration::from_secs(3),
                rpc(socket, token, method),
            )
            .await
            .ok()
            .flatten()
        });
        if let Some(quota) = response.and_then(|v| parse_rpc(method, &v)) {
            return Some(quota);
        }
    }
    None
}

async fn rpc(socket: std::net::TcpStream, token: String, method: &str) -> Option<Value> {
    use http_body_util::{BodyExt, Full, Limited};
    use hyper::{Request, body::Bytes, client::conn::http1};
    let host = socket.peer_addr().ok()?;
    socket.set_nonblocking(true).ok()?;
    let io = hyper_util::rt::TokioIo::new(tokio::net::TcpStream::from_std(socket).ok()?);
    let (mut sender, connection) = http1::handshake(io).await.ok()?;
    tokio::spawn(connection);
    let request = Request::post(format!(
        "/exa.language_server_pb.LanguageServerService/{method}"
    ))
    .header("host", host.to_string())
    .header("content-type", "application/json")
    .header("connect-protocol-version", "1")
    .header("x-codeium-csrf-token", token)
    .body(Full::new(Bytes::from_static(b"{}")))
    .ok()?;
    let response = sender.send_request(request).await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = Limited::new(response.into_body(), LIMIT as usize)
        .collect()
        .await
        .ok()?
        .to_bytes();
    serde_json::from_slice(&bytes).ok()
}

fn parse_rpc(method: &str, doc: &Value) -> Option<AntigravityQuota> {
    let mut groups = Groups::default();
    if method == "RetrieveUserQuotaSummary" {
        let summary = &doc["response"];
        let mut add = |group: &str, buckets: &Value| {
            for b in buckets.as_array().into_iter().flatten() {
                if b["disabled"] == true {
                    continue;
                }
                let label = format!(
                    "{group} {} {} {}",
                    b["bucketId"].as_str().unwrap_or_default(),
                    b["displayName"].as_str().unwrap_or_default(),
                    b["window"].as_str().unwrap_or_default()
                );
                if let (Some(f), Some(t)) =
                    (b["remainingFraction"].as_f64(), reset(&b["resetTime"]))
                {
                    groups.add(&label, &label, f, t);
                }
            }
        };
        for group in summary["groups"].as_array().into_iter().flatten() {
            add(
                group["displayName"].as_str().unwrap_or_default(),
                &group["buckets"],
            );
        }
        add("", &summary["buckets"]);
    } else {
        let configs = doc
            .pointer("/userStatus/cascadeModelConfigData/clientModelConfigs")?
            .as_array()?;
        for model in configs {
            if model["disabled"] == true {
                continue;
            }
            let info = &model["quotaInfo"];
            let Some(at) = reset(&info["resetTime"]) else {
                continue;
            };
            // In this legacy proto only, an omitted scalar has the protobuf default zero.
            let f = match info.get("remainingFraction") {
                None => 0.,
                Some(v) => match v.as_f64() {
                    Some(f) => f,
                    None => continue,
                },
            };
            let label = format!(
                "{} {} {}",
                model["label"].as_str().unwrap_or_default(),
                model["modelId"].as_str().unwrap_or_default(),
                model["provider"].as_str().unwrap_or_default()
            );
            groups.add(&label, &label, f, at);
        }
    }
    groups.finish()
}

#[cfg(target_os = "macos")]
mod discovery;

pub fn observe(
    path: Option<&Path>,
    previous: Option<&AntigravityQuota>,
) -> (Source<AntigravityQuota>, Option<&'static str>) {
    #[cfg(target_os = "macos")]
    let live = discovery::read();
    #[cfg(not(target_os = "macos"))]
    let live = Err("Antigravity local RPC requires macOS");
    let reason = match live {
        Ok(quota) => return (Source::Current(quota), None),
        Err(reason) => reason,
    };
    if let Some(path) = path {
        match snapshot(path) {
            Some(source @ Source::Current(_)) => return (source, None),
            Some(source) => return (source, Some("Antigravity statusLine capture is stale")),
            None => {
                return (
                    observation(None, previous),
                    Some("Antigravity statusLine capture unavailable or unrecognized"),
                );
            }
        }
    }
    (observation(None, previous), Some(reason))
}
