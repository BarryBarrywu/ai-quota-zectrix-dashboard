use crate::model::{CodexQuota, QuotaWindow, Source, WindowKind};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodexFailure {
    Transport,
    Rejected,
    InvalidQuota,
    NoWeeklyWindow,
}

pub struct CodexObserver {
    failure: Option<CodexFailure>,
    program: PathBuf,
    session: Option<Session>,
    cached: Option<CodexQuota>,
    clock: Box<dyn Fn() -> Duration + Send>,
    reconnect_at: Duration,
    delay: Duration,
}

impl CodexObserver {
    pub fn new(program: impl AsRef<Path>) -> Self {
        let start = Instant::now();
        Self::with_clock(program, move || start.elapsed())
    }

    pub fn with_clock(
        program: impl AsRef<Path>,
        clock: impl Fn() -> Duration + Send + 'static,
    ) -> Self {
        Self {
            failure: None,
            program: program.as_ref().into(),
            session: None,
            cached: None,
            clock: Box::new(clock),
            reconnect_at: Duration::ZERO,
            delay: Duration::from_secs(1),
        }
    }

    pub fn restore_cache(&mut self, cached: CodexQuota) {
        if cached.weekly.kind == WindowKind::Weekly && cached.weekly.remaining_percent <= 100 {
            self.cached = Some(cached);
        }
    }

    pub fn last_failure(&self) -> Option<CodexFailure> {
        self.failure
    }

    pub fn poll(&mut self) -> Source<CodexQuota> {
        if self.session.is_none() {
            if (self.clock)() < self.reconnect_at {
                return self.stale();
            }
            match Session::start(&self.program) {
                Ok(session) => self.session = Some(session),
                Err(reason) => return self.disconnected(reason),
            }
        }
        let session = self.session.as_mut().unwrap();
        let result = match session.request("account/rateLimits/read", Value::Null) {
            Ok(value) => value,
            Err(CodexFailure::Rejected) => {
                self.failure = Some(CodexFailure::Rejected);
                self.delay = Duration::from_secs(1);
                return self.stale();
            }
            Err(reason) => return self.disconnected(reason),
        };
        self.delay = Duration::from_secs(1);
        match parse_quota(result) {
            Ok(quota) => {
                self.failure = quota.is_none().then_some(CodexFailure::NoWeeklyWindow);
                self.cached = quota.clone();
                quota.map(Source::Current).unwrap_or_default()
            }
            Err(()) => {
                self.failure = Some(CodexFailure::InvalidQuota);
                self.stale()
            }
        }
    }

    fn disconnected(&mut self, reason: CodexFailure) -> Source<CodexQuota> {
        self.failure = Some(reason);
        self.session = None;
        self.reconnect_at = (self.clock)().saturating_add(self.delay);
        self.delay = (self.delay * 2).min(Duration::from_secs(60));
        self.stale()
    }

    fn stale(&self) -> Source<CodexQuota> {
        self.cached.clone().map(Source::Stale).unwrap_or_default()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    rate_limits: Limits,
    rate_limit_reset_credits: Option<Credits>,
}
#[derive(Deserialize)]
struct Limits {
    primary: Window,
    secondary: Option<Window>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Credits {
    available_count: u32,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Window {
    used_percent: u8,
    window_duration_mins: u64,
    resets_at: i64,
}

fn parse_quota(value: Value) -> Result<Option<CodexQuota>, ()> {
    let response: Response = serde_json::from_value(value).map_err(|_| ())?;
    let limits = response.rate_limits;
    let mut weekly = None;
    for window in std::iter::once(limits.primary).chain(limits.secondary) {
        if window.window_duration_mins == 0 || window.used_percent > 100 {
            return Err(());
        }
        if window.window_duration_mins == 7 * 24 * 60 {
            weekly = Some(QuotaWindow {
                kind: WindowKind::Weekly,
                remaining_percent: 100 - window.used_percent,
                resets_at: window.resets_at,
            });
        }
    }
    Ok(weekly.map(|weekly| CodexQuota {
        weekly,
        reset_credits: response
            .rate_limit_reset_credits
            .map(|credits| credits.available_count),
    }))
}

struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    messages: Receiver<std::io::Result<String>>,
    next_id: u64,
}
impl Session {
    fn start(program: &Path) -> Result<Self, CodexFailure> {
        let mut child = Command::new(program)
            .args(["app-server", "--stdio"])
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| CodexFailure::Transport)?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, messages) = mpsc::sync_channel(16);
        std::thread::spawn(move || {
            for message in BufReader::new(stdout).lines() {
                if sender.send(message).is_err() {
                    break;
                }
            }
        });
        let mut session = Self {
            child,
            stdin: Some(stdin),
            messages,
            next_id: 1,
        };
        session.request("initialize", json!({"clientInfo":{"name":"ai-quota-zectrix-dashboard","version":env!("CARGO_PKG_VERSION")}}))?;
        writeln!(
            session.stdin.as_mut().unwrap(),
            "{}",
            json!({"method":"initialized"})
        )
        .map_err(|_| CodexFailure::Transport)?;
        Ok(session)
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, CodexFailure> {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.stdin.as_mut().unwrap(),
            "{}",
            json!({"id":id,"method":method,"params":params})
        )
        .map_err(|_| CodexFailure::Transport)?;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or(CodexFailure::Transport)?;
            let line = self
                .messages
                .recv_timeout(remaining)
                .map_err(|_| CodexFailure::Transport)?
                .map_err(|_| CodexFailure::Transport)?;
            let message: Value =
                serde_json::from_str(&line).map_err(|_| CodexFailure::Transport)?;
            if message["id"].as_u64() == Some(id) && message.get("method").is_none() {
                if message.get("error").is_some() {
                    return Err(CodexFailure::Rejected);
                }
                return message
                    .get("result")
                    .cloned()
                    .ok_or(CodexFailure::Transport);
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(1);
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            if self.messages.recv_timeout(remaining).is_err() {
                break;
            }
        }
        // The child leads its own process group and its PID cannot be recycled until wait().
        unsafe {
            libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}
