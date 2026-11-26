use std::{
    net::{SocketAddr, TcpStream},
    process::Command,
    time::Duration,
};

const PROGRAM: &str = "/Applications/Antigravity.app/Contents/Resources/bin/language_server";
const REQUIREMENT: &str = r#"=anchor apple generic and identifier "language_server" and certificate leaf[subject.OU] = "EQHXZ8M8AV""#;

fn output(program: &str, args: &[&str]) -> Option<String> {
    let run = Command::new(program).args(args).output().ok()?;
    run.status
        .success()
        .then(|| String::from_utf8(run.stdout).ok())?
}

fn signed(pid: &str, uid: &str) -> bool {
    let identity = output("/bin/ps", &["-ww", "-p", pid, "-o", "uid=,comm="]);
    identity
        .as_deref()
        .is_some_and(|id| id.split_whitespace().eq([uid, PROGRAM]))
        && output("/usr/bin/codesign", &["--verify", "-R", REQUIREMENT, pid]).is_some()
}

fn sockets(pid: &str, uid: &str, state: &str) -> Option<String> {
    output(
        "/usr/sbin/lsof",
        &["-nP", "-a", "-p", pid, "-u", uid, "-iTCP", state, "-Fn"],
    )
}

fn flag<'a>(command: &'a str, name: &str) -> Option<&'a str> {
    let mut words = command.split_ascii_whitespace();
    while let Some(word) = words.next() {
        if word == name {
            return words.next();
        }
        if let Some(value) = word
            .strip_prefix(name)
            .and_then(|suffix| suffix.strip_prefix('='))
        {
            return Some(value);
        }
    }
    None
}

fn endpoint<'a>(command: &'a str, listeners: &str) -> Option<(SocketAddr, &'a str)> {
    let token = flag(command, "--csrf_token").filter(|s| {
        !s.starts_with("--")
            && !s.is_empty()
            && s.len() <= 256
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    })?;
    let port = ["--http_port", "--http_server_port"]
        .iter()
        .find_map(|name| flag(command, name)?.parse::<u16>().ok())?;
    let addr = listeners
        .lines()
        .filter_map(|s| s.strip_prefix('n')?.parse::<SocketAddr>().ok())
        .find(|a| a.ip().is_loopback() && a.port() == port)?;
    Some((addr, token))
}

pub(super) struct LocalServer {
    pid: String,
    uid: String,
    addr: SocketAddr,
    token: String,
}

impl LocalServer {
    pub(super) fn discover() -> Result<Self, &'static str> {
        let uid = unsafe { libc::geteuid() }.to_string();
        let pids = output("/usr/bin/pgrep", &["-x", "-u", &uid, "language_server"])
            .ok_or("Antigravity language server unavailable")?;
        for pid in pids.lines().filter(|s| s.parse::<u32>().is_ok()) {
            if !signed(pid, &uid) {
                continue;
            }
            let Some(command) = output("/bin/ps", &["-ww", "-p", pid, "-o", "command="]) else {
                continue;
            };
            let Some(listeners) = sockets(pid, &uid, "-sTCP:LISTEN") else {
                continue;
            };
            if let Some((addr, token)) = endpoint(&command, &listeners) {
                return Ok(Self {
                    pid: pid.to_owned(),
                    uid,
                    addr,
                    token: token.to_owned(),
                });
            }
        }
        Err("No supported signed Antigravity HTTP quota endpoint")
    }

    pub(super) fn connect(&self) -> Option<(TcpStream, String)> {
        let socket = TcpStream::connect_timeout(&self.addr, Duration::from_secs(1)).ok()?;
        let expected = format!("n{}->{}", self.addr, socket.local_addr().ok()?);
        // Pin the already-connected socket to the verified process before attaching credentials.
        if !sockets(&self.pid, &self.uid, "-sTCP:ESTABLISHED")?
            .lines()
            .any(|line| line == expected)
            || !signed(&self.pid, &self.uid)
        {
            return None;
        }
        Some((socket, self.token.clone()))
    }
}

pub(super) fn read() -> Result<crate::model::AntigravityQuota, &'static str> {
    let endpoint = LocalServer::discover()?;
    super::query(|| endpoint.connect()).ok_or("Antigravity quota RPC unavailable or unrecognized")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsigned_local_listener_receives_no_credentials() {
        use std::io::Read;
        use std::net::TcpListener;
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let server = LocalServer {
            pid: std::process::id().to_string(),
            uid: unsafe { libc::geteuid() }.to_string(),
            addr: listener.local_addr().unwrap(),
            token: String::from("test-token"),
        };
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            socket.read_to_end(&mut bytes).unwrap();
            bytes
        });
        assert!(server.connect().is_none());
        assert!(thread.join().unwrap().is_empty());
    }

    #[test]
    fn discovery_considers_only_declared_http_port_on_an_owned_loopback_listener() {
        let cmd =
            "language_server --csrf_token test --https_server_port 5021 --http_server_port=5022";
        assert_eq!(
            endpoint(cmd, "n127.0.0.1:5021\nn127.0.0.1:5022")
                .unwrap()
                .0
                .port(),
            5022
        );
        assert!(endpoint(cmd, "n*:5022\nn127.0.0.1:5021").is_none());
        assert!(
            endpoint(
                "language_server --https_server_port 5021",
                "n127.0.0.1:5021"
            )
            .is_none()
        );
    }
}
