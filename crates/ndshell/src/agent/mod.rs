//! The Windows agent (ARCHITECTURE.md 10.2). Processes OpenSSH starts have no interactive
//! desktop: they can neither show a window nor post input to one. `ndshell agent` runs in the
//! logged-in user's session instead (installed per user, started at logon and on install by Task
//! Scheduler with `/IT`), and tier 3 runs send it jobs: launch an app, post `SendInput`, inject
//! pen input, answer hit tests, and capture windows and monitors with Windows.Graphics.Capture.
//!
//! Transport: JSON lines over TCP bound to 127.0.0.1 only, so the macbook reaches it through
//! `ssh -L 47811:127.0.0.1:47811 windows`. The first line must carry the token written at
//! install (`%LOCALAPPDATA%\nd\agent\token`, readable only by the user's profile); anything else
//! closes the connection. One connection is served at a time, which also serialises real input.
//!
//! Request: `{"cmd": "<name>", ...}`. Reply: `{"ok": true, ...}` or `{"ok": false, "error": ".."}`.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod input;
#[cfg(windows)]
pub mod install;
#[cfg(windows)]
pub mod server;

pub const DEFAULT_PORT: u16 = 47811;

/// `ND_AGENT_ADDR`, else `127.0.0.1:47811`.
pub fn addr() -> String {
    std::env::var("ND_AGENT_ADDR").unwrap_or_else(|_| format!("127.0.0.1:{DEFAULT_PORT}"))
}

/// `%LOCALAPPDATA%\nd\agent`: the installed binary, token and log.
pub fn home() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("nd").join("agent")
}

pub fn token_path() -> PathBuf {
    home().join("token")
}

/// `ND_AGENT_TOKEN`, else the token file (present on the Windows machine itself).
pub fn token() -> Result<String, String> {
    if let Ok(token) = std::env::var("ND_AGENT_TOKEN") {
        return Ok(token.trim().to_owned());
    }
    let path = token_path();
    std::fs::read_to_string(&path).map(|t| t.trim().to_owned()).map_err(|e| {
        format!("no agent token at {} ({e}); run `ndshell agent install` there, or set ND_AGENT_TOKEN", path.display())
    })
}

/// A connection to the agent.
pub struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl Client {
    pub fn connect() -> Result<Self, String> {
        Self::connect_to(&addr(), &token()?)
    }

    pub fn connect_to(addr: &str, token: &str) -> Result<Self, String> {
        let stream = TcpStream::connect(addr).map_err(|e| {
            format!(
                "agent at {addr}: {e}; is `ndshell agent` running in the desktop session (`ndshell agent install`)?"
            )
        })?;
        stream.set_nodelay(true).ok();
        let writer = stream.try_clone().map_err(|e| e.to_string())?;
        let mut client = Self { reader: BufReader::new(stream), writer };
        client.call(json!({ "token": token }))?;
        Ok(client)
    }

    /// Sends one request and waits for its reply; an `ok: false` reply is the error.
    pub fn call(&mut self, request: Value) -> Result<Value, String> {
        let mut line = request.to_string();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).map_err(|e| format!("agent write: {e}"))?;
        let mut reply = String::new();
        let n = self.reader.read_line(&mut reply).map_err(|e| format!("agent read: {e}"))?;
        if n == 0 {
            return Err("agent closed the connection".into());
        }
        let reply: Value = serde_json::from_str(&reply).map_err(|e| format!("agent reply: {e}: {reply}"))?;
        if reply["ok"] == true { Ok(reply) } else { Err(reply["error"].as_str().unwrap_or("agent error").to_owned()) }
    }

    pub fn set_timeout(&self, timeout: Option<Duration>) {
        let _ = self.writer.set_read_timeout(timeout);
    }
}

/// Constant-time comparison, so a wrong token learns nothing from timing.
pub fn token_matches(given: &str, expected: &str) -> bool {
    let (a, b) = (given.as_bytes(), expected.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_comparison() {
        assert!(token_matches("abc123", "abc123"));
        assert!(!token_matches("abc124", "abc123"));
        assert!(!token_matches("abc12", "abc123"));
        assert!(!token_matches("", "abc123"));
    }
}
