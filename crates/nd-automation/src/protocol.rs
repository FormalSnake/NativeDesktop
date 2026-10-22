//! Wire format: JSON-RPC 2.0, each message a little-endian `u32` byte length followed by that
//! many bytes of UTF-8 JSON. The client side lives in `nd-test`; this is the one definition of
//! the framing and the error codes.

use std::fmt;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Frames above this are refused rather than allocated.
pub const MAX_FRAME: u32 = 64 << 20;

pub fn write_frame(w: &mut impl Write, value: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(value)?;
    let len = u32::try_from(body.len()).map_err(|_| io::Error::other("frame too large"))?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(&body)?;
    w.flush()
}

/// `Ok(None)` on a clean end of stream between frames.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Value>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len);
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("frame of {len} bytes")));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body).map(Some).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// The error codes of the old `schema/rpc.json`, unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpcErrorCode {
    /// The target is gone, invisible, disabled or does not take the action. `data`:
    /// `{ref, reason}`.
    NotActionable = -32001,
    /// `waitFor` ran out of time. `data`: `{timeoutMs}`.
    WaitForTimeout = -32002,
    /// The backend cannot synthesize input events.
    InputUnsupported = -32003,
    /// The window named by `window` closed. `data`: `{window}`.
    WindowGone = -32004,
    MethodNotFound = -32601,
    InvalidParams = -32602,
    InternalError = -32603,
    ParseError = -32700,
}

impl RpcErrorCode {
    pub fn from_code(code: i64) -> Option<Self> {
        use RpcErrorCode::*;
        [
            NotActionable,
            WaitForTimeout,
            InputUnsupported,
            WindowGone,
            MethodNotFound,
            InvalidParams,
            InternalError,
            ParseError,
        ]
        .into_iter()
        .find(|c| *c as i64 == code)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: RpcErrorCode, message: impl Into<String>) -> Self {
        Self { code: code as i64, message: message.into(), data: None }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    pub fn kind(&self) -> Option<RpcErrorCode> {
        RpcErrorCode::from_code(self.code)
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(RpcErrorCode::InvalidParams, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(RpcErrorCode::InternalError, message)
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)?;
        if let Some(data) = &self.data {
            write!(f, " {data}")?;
        }
        Ok(())
    }
}

impl std::error::Error for RpcError {}

pub fn request(id: u64, method: &str, params: Value) -> Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

pub fn response(id: Value, result: Result<Value, RpcError>) -> Value {
    match result {
        Ok(result) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    }
}

/// The result or error of a response frame.
pub fn parse_response(frame: Value) -> Result<(Value, Result<Value, RpcError>), RpcError> {
    let id = frame.get("id").cloned().unwrap_or(Value::Null);
    if let Some(error) = frame.get("error") {
        let error: RpcError = serde_json::from_value(error.clone())
            .map_err(|e| RpcError::new(RpcErrorCode::ParseError, format!("malformed error: {e}")))?;
        return Ok((id, Err(error)));
    }
    let result = frame.get("result").cloned().unwrap_or(Value::Null);
    Ok((id, Ok(result)))
}

/// The named pipe a socket path stands for on Windows: the path itself when it already names a
/// pipe (`\\.\pipe\..`), else a pipe named after it, so `ND_AUTOMATION_SOCKET` and the
/// per-pid default keep working unchanged.
pub fn pipe_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text.starts_with(r"\\.\pipe\") {
        return path.to_owned();
    }
    // FNV-1a: stable across processes, unlike `DefaultHasher`.
    let hash = text.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3));
    PathBuf::from(format!(r"\\.\pipe\nd-automation-{hash:016x}"))
}

/// A client's connection to an app's automation server.
#[cfg(unix)]
pub type Stream = std::os::unix::net::UnixStream;
#[cfg(windows)]
pub type Stream = interprocess::local_socket::Stream;

/// Connects to the server listening on `path`: the Unix socket itself, or on Windows the named
/// pipe it stands for ([`pipe_path`]).
#[cfg(unix)]
pub fn connect(path: &Path) -> io::Result<Stream> {
    Stream::connect(path)
}

#[cfg(windows)]
pub fn connect(path: &Path) -> io::Result<Stream> {
    use interprocess::local_socket::GenericFilePath;
    use interprocess::local_socket::prelude::*;
    let pipe = pipe_path(path);
    Stream::connect(pipe.as_path().to_fs_name::<GenericFilePath>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_end_cleanly() {
        let mut buf = Vec::new();
        write_frame(&mut buf, &request(7, "getTree", serde_json::json!({}))).unwrap();
        assert_eq!(&buf[..4], &(buf.len() as u32 - 4).to_le_bytes());
        let mut r = &buf[..];
        let frame = read_frame(&mut r).unwrap().unwrap();
        assert_eq!(frame["method"], "getTree");
        assert!(read_frame(&mut r).unwrap().is_none());
    }

    #[test]
    fn errors_keep_their_codes() {
        let e = RpcError::new(RpcErrorCode::WaitForTimeout, "waitFor timeout")
            .with_data(serde_json::json!({"timeoutMs": 5}));
        let (_, result) = parse_response(response(Value::from(1), Err(e.clone()))).unwrap();
        assert_eq!(result.unwrap_err(), e);
        assert_eq!(e.kind(), Some(RpcErrorCode::WaitForTimeout));
    }

    #[test]
    fn pipe_names_are_stable_and_pipes_pass_through() {
        let pipe = pipe_path(Path::new("/tmp/nd-docs-1-light.sock"));
        assert_eq!(pipe, pipe_path(Path::new("/tmp/nd-docs-1-light.sock")));
        assert!(pipe.to_string_lossy().starts_with(r"\\.\pipe\nd-automation-"));
        assert_ne!(pipe, pipe_path(Path::new("/tmp/nd-docs-1-dark.sock")));
        assert_eq!(pipe_path(&pipe), pipe);
    }
}
