//! Native crash capture, Zed's `crashes` pattern: the app re-launches itself as a minidump
//! server (`<exe> --nd-crash-handler <socket> <diagnostics dir>`) and installs crash-handler's
//! exception port, which asks the server for a dump of the whole process. A crash dumps into a
//! new `-crash` bundle, filled from the snapshot the app last wrote; a panic or a hang asks for a
//! dump into the bundle it already wrote.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crash_handler::{CrashEventResult, CrashHandler};
use minidumper::{LoopAction, MinidumpBinary, Server, ServerHandler, SocketName};
use serde_json::json;

use crate::bundle::{self, Kind, Retention, Snapshot};

pub const ARG: &str = "--nd-crash-handler";
pub const SNAPSHOT: &str = ".snapshot.json";
const DUMP_TO: u32 = 1;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Exit status after a native crash was dumped. The process exits itself rather than letting
/// the OS crash reporter take over and show its dialog.
const CRASH_EXIT: i32 = 70;

pub(crate) struct CrashClient {
    client: Arc<minidumper::Client>,
    handler: CrashHandler,
    /// Serializes `DUMP_TO` and the dump it names.
    lock: Mutex<()>,
    _server: Child,
}

impl CrashClient {
    /// Spawns the server and connects to it. Blocks until connected, so callers run it off the
    /// main thread.
    pub fn start(diagnostics: &Path) -> io::Result<Self> {
        let socket = std::env::temp_dir().join(format!("nd-crash-{}.sock", std::process::id()));
        let _ = fs::remove_file(&socket);
        let server = Command::new(std::env::current_exe()?)
            .arg(ARG)
            .arg(&socket)
            .arg(diagnostics)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let start = Instant::now();
        let client = loop {
            match minidumper::Client::with_name(SocketName::Path(&socket)) {
                Ok(client) => break Arc::new(client),
                Err(_) if start.elapsed() < CONNECT_TIMEOUT => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => return Err(io::Error::other(e)),
            }
        };
        let dumper = client.clone();
        // SAFETY: the closure runs on crash-handler's exception thread with every other thread
        // suspended; it only talks to the server over the already connected socket.
        let handler = CrashHandler::attach(unsafe {
            crash_handler::make_crash_event(move |cx: &crash_handler::CrashContext| {
                let dumped = dumper.request_dump(cx).is_ok();
                if is_real_crash(cx) {
                    libc::_exit(CRASH_EXIT);
                }
                CrashEventResult::Handled(dumped)
            })
        })
        .map_err(io::Error::other)?;
        Ok(Self { client, handler, lock: Mutex::new(()), _server: server })
    }

    /// Writes a minidump of the running process to `<dir>/minidump.dmp`.
    pub fn dump_into(&self, dir: &Path) -> bool {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let path = dir.as_os_str().as_encoded_bytes();
        if self.client.send_message(DUMP_TO, path).is_err() {
            return false;
        }
        // The server answers pings in order, so the path has landed before the dump request.
        self.client.ping().is_ok() && simulate(&self.handler)
    }
}

/// A real exception, as opposed to the simulated ones panics and hangs dump through.
#[cfg(target_os = "macos")]
fn is_real_crash(cx: &crash_handler::CrashContext) -> bool {
    cx.exception.is_some()
}

/// A real signal; a simulated one is sent by the process itself (`SI_USER`).
#[cfg(target_os = "linux")]
fn is_real_crash(cx: &crash_handler::CrashContext) -> bool {
    cx.siginfo.ssi_code != 0
}

#[cfg(target_os = "macos")]
fn simulate(handler: &CrashHandler) -> bool {
    handler.simulate_exception(None)
}

#[cfg(target_os = "linux")]
fn simulate(handler: &CrashHandler) -> bool {
    matches!(handler.simulate_signal(libc::SIGUSR1 as u32), CrashEventResult::Handled(true))
}

struct Handler {
    diagnostics: PathBuf,
    target: Mutex<Option<PathBuf>>,
    current: Mutex<Option<(PathBuf, bool)>>,
    connected: Arc<AtomicBool>,
}

impl ServerHandler for Handler {
    fn create_minidump_file(&self) -> io::Result<(File, PathBuf)> {
        let (dir, crash) = match self.target.lock().expect("target").take() {
            Some(dir) => (dir, false),
            None => (bundle::create_dir(&self.diagnostics, Kind::Crash)?, true),
        };
        let path = dir.join("minidump.dmp");
        *self.current.lock().expect("current") = Some((dir, crash));
        Ok((File::create(&path)?, path))
    }

    fn on_minidump_created(&self, result: Result<MinidumpBinary, minidumper::Error>) -> LoopAction {
        let Some((dir, crash)) = self.current.lock().expect("current").take() else { return LoopAction::Continue };
        let error = result.err().map(|e| e.to_string());
        if let Some(error) = &error {
            let _ = fs::write(dir.join("minidump-error.txt"), error);
        }
        if !crash {
            return LoopAction::Continue;
        }
        let snapshot: Snapshot = fs::read(self.diagnostics.join(SNAPSHOT))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let _ = bundle::write_snapshot(&dir, &snapshot);
        let crash = json!({ "kind": "native", "minidump": error.is_none(), "error": error });
        let _ = fs::write(dir.join("crash.json"), serde_json::to_vec_pretty(&crash).unwrap_or_default());
        let _ = bundle::enforce_retention(&self.diagnostics, Retention::default());
        LoopAction::Exit
    }

    fn on_message(&self, kind: u32, buffer: Vec<u8>) {
        if kind == DUMP_TO {
            *self.target.lock().expect("target") = Some(PathBuf::from(String::from_utf8_lossy(&buffer).into_owned()));
        }
    }

    fn on_client_connected(&self, _clients: usize) -> LoopAction {
        self.connected.store(true, Ordering::SeqCst);
        LoopAction::Continue
    }

    fn on_client_disconnected(&self, clients: usize) -> LoopAction {
        if clients == 0 { LoopAction::Exit } else { LoopAction::Continue }
    }
}

/// The server side, run by the re-launched executable. Returns when the app disconnects, has
/// crashed, or never connected.
pub fn run_server(socket: &Path, diagnostics: &Path) {
    let Ok(mut server) = Server::with_name(SocketName::Path(socket)) else { return };
    let shutdown = Arc::new(AtomicBool::new(false));
    let connected = Arc::new(AtomicBool::new(false));
    {
        let (shutdown, connected) = (shutdown.clone(), connected.clone());
        // SAFETY: getppid has no preconditions.
        let parent = unsafe { libc::getppid() };
        std::thread::spawn(move || {
            let start = Instant::now();
            // The app exiting does not always surface as a disconnect, so also stop once the
            // process is reparented.
            while !shutdown.load(Ordering::SeqCst) {
                let orphaned = unsafe { libc::getppid() } != parent;
                let never_connected = start.elapsed() > CONNECT_TIMEOUT && !connected.load(Ordering::SeqCst);
                if orphaned || never_connected {
                    shutdown.store(true, Ordering::SeqCst);
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        });
    }
    let handler =
        Handler { diagnostics: diagnostics.to_owned(), target: Mutex::new(None), current: Mutex::new(None), connected };
    let _ = server.run(Box::new(handler), &shutdown, None);
    let _ = fs::remove_file(socket);
}
