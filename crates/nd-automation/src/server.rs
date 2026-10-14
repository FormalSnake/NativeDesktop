//! The socket server. A listener thread accepts connections, a thread per connection reads
//! requests in order, and each request runs on the app's thread through the [`Executor`] the
//! host provides. Nothing here blocks the app's thread: `waitFor` sleeps on the connection
//! thread between checks.
//!
//! Environment: `NATIVE_AUTOMATION=1` enables it; `ND_AUTOMATION_SOCKET` sets the socket path
//! (default `$TMPDIR/nd-automation-<pid>.sock`). Once listening the server prints
//! `ND_AUTOMATION_LISTENING path=<path>` on stderr, which launchers wait for.
// Without unix sockets (the web, Windows until named pipes) only `start`'s error is compiled in.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use nd_ui::App;
use serde_json::{Value, json};

use crate::handler::{WaitParams, handle, wait_check};
use crate::protocol::{RpcError, RpcErrorCode, read_frame, response, write_frame};

/// Work for the app's thread.
pub type Job = Box<dyn FnOnce(&mut App) + Send>;

/// Runs a [`Job`] on the thread that owns the app, soon, outside any frame or entity lease. The
/// AppKit host's `AppSender` is one.
pub trait Executor: Send + Sync + 'static {
    fn execute(&self, job: Job);
}

impl<F: Fn(Job) + Send + Sync + 'static> Executor for F {
    fn execute(&self, job: Job) {
        self(job)
    }
}

/// How long one request may wait for the app's thread before the reply says it is hung.
const APP_THREAD_TIMEOUT: Duration = Duration::from_secs(30);
const WAIT_TICK: Duration = Duration::from_millis(50);

fn on_app<T: Send + 'static>(
    exec: &dyn Executor,
    f: impl FnOnce(&mut App) -> T + Send + 'static,
) -> Result<T, RpcError> {
    let (tx, rx) = mpsc::sync_channel(1);
    exec.execute(Box::new(move |app| {
        let _ = tx.send(f(app));
    }));
    rx.recv_timeout(APP_THREAD_TIMEOUT)
        .map_err(|_| RpcError::internal(format!("the app's thread did not answer within {APP_THREAD_TIMEOUT:?}")))
}

fn wait_for(exec: &dyn Executor, params: &Value) -> Result<Value, RpcError> {
    let wait = WaitParams::parse(params)?;
    let timeout = Duration::from_millis(wait.timeout_ms());
    let start = Instant::now();
    loop {
        let check = wait.clone();
        if let Some(mut result) = on_app(exec, move |app| wait_check(app, &check))?? {
            result["elapsedMs"] = json!(start.elapsed().as_millis() as u64);
            return Ok(result);
        }
        if start.elapsed() >= timeout {
            return Err(RpcError::new(RpcErrorCode::WaitForTimeout, "waitFor timeout")
                .with_data(json!({ "timeoutMs": wait.timeout_ms() })));
        }
        std::thread::sleep(WAIT_TICK);
    }
}

fn dispatch(exec: &dyn Executor, request: &Value) -> Result<Value, RpcError> {
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::new(RpcErrorCode::ParseError, "request has no method"))?
        .to_owned();
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    if method == "waitFor" {
        return wait_for(exec, &params);
    }
    on_app(exec, move |app| handle(app, &method, &params))?
}

#[cfg(unix)]
fn serve(stream: std::os::unix::net::UnixStream, exec: Arc<dyn Executor>) {
    let mut reader = match stream.try_clone() {
        Ok(r) => r,
        Err(_) => return,
    };
    let mut writer = stream;
    while let Ok(Some(request)) = read_frame(&mut reader) {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let reply = response(id, dispatch(&*exec, &request));
        if write_frame(&mut writer, &reply).is_err() {
            return;
        }
    }
}

/// Listens on `path` and serves requests through `exec` until the process exits.
#[cfg(unix)]
pub fn start(path: &Path, exec: impl Executor) -> io::Result<PathBuf> {
    use std::os::unix::net::UnixListener;
    // A socket left by a crashed run would make bind fail.
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    let exec: Arc<dyn Executor> = Arc::new(exec);
    std::thread::Builder::new().name("nd-automation".into()).spawn(move || {
        for stream in listener.incoming().map_while(Result::ok) {
            let exec = exec.clone();
            let _ = std::thread::Builder::new().name("nd-automation-conn".into()).spawn(move || serve(stream, exec));
        }
    })?;
    eprintln!("ND_AUTOMATION_LISTENING path={}", path.display());
    Ok(path.to_owned())
}

#[cfg(not(unix))]
pub fn start(_path: &Path, _exec: impl Executor) -> io::Result<PathBuf> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "the automation socket needs unix sockets (named pipes land with the Windows backend)",
    ))
}

/// Starts the server when `NATIVE_AUTOMATION=1`. `Ok(None)` when it is not enabled.
pub fn start_from_env(exec: impl Executor) -> io::Result<Option<PathBuf>> {
    if std::env::var_os("NATIVE_AUTOMATION").is_none_or(|v| v != "1") {
        return Ok(None);
    }
    let path = std::env::var_os("ND_AUTOMATION_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("nd-automation-{}.sock", std::process::id())));
    start(&path, exec).map(Some)
}
