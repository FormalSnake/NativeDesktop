//! Tier 3: the app as its own process, driven over its automation socket.
//!
//! Every launch gets its own directory (socket, `ND_APP_DATA_DIR`), so parallel launches never
//! share a path; the child reports its socket on stderr (`ND_AUTOMATION_LISTENING path=..`).
//! On macOS a launched AppKit app activates itself and takes focus, so a launch holds the
//! machine-wide real-input lock (`ndshell::lock`) until the [`AppHandle`] drops: concurrent
//! tier 3 runs, in this worktree or another, queue instead of stealing focus from each other.

use std::cell::{Cell, RefCell};
use std::io::{BufRead, BufReader};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nd_automation::protocol::{parse_response, read_frame, request, write_frame};
use serde_json::Value;

use crate::session::{Driver, Session};
use crate::{Error, Result};

const READY_MARKER: &str = "ND_AUTOMATION_LISTENING path=";
const POLL: Duration = Duration::from_millis(50);

/// The built binary of a workspace package: `$ND_<NAME>_BIN` if set, else
/// `$CARGO_TARGET_DIR` (or the workspace's `target/`) `/debug/<name>`.
pub fn example_bin(name: &str) -> PathBuf {
    let var = format!("ND_{}_BIN", name.to_uppercase().replace('-', "_"));
    if let Some(bin) = std::env::var_os(var) {
        return PathBuf::from(bin);
    }
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    target.join("debug").join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

#[derive(Clone, Debug)]
pub struct LaunchOptions {
    pub bin: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// How long to wait for the socket marker.
    pub ready_timeout: Duration,
    /// Hold the machine-wide real-input lock for the app's life. On by default on macOS.
    pub machine_lock: bool,
}

impl LaunchOptions {
    pub fn new(bin: impl Into<PathBuf>) -> Self {
        Self {
            bin: bin.into(),
            args: Vec::new(),
            env: Vec::new(),
            ready_timeout: Duration::from_secs(20),
            machine_lock: cfg!(target_os = "macos"),
        }
    }

    /// A binary of this workspace, by package name (see [`example_bin`]).
    pub fn example(name: &str) -> Self {
        Self::new(example_bin(name))
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn machine_lock(mut self, on: bool) -> Self {
        self.machine_lock = on;
        self
    }
}

struct SocketDriver {
    stream: RefCell<nd_automation::protocol::Stream>,
    next_id: Cell<u64>,
}

impl SocketDriver {
    fn connect(path: &Path) -> Result<Self> {
        let stream = nd_automation::protocol::connect(path)
            .map_err(|e| Error::Launch(format!("connect {}: {e}", path.display())))?;
        Ok(Self { stream: RefCell::new(stream), next_id: Cell::new(1) })
    }
}

impl Driver for SocketDriver {
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.replace(self.next_id.get() + 1);
        let mut stream = self.stream.borrow_mut();
        write_frame(&mut *stream, &request(id, method, params))?;
        loop {
            let frame = read_frame(&mut *stream)?.ok_or_else(|| {
                Error::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the app closed the socket"))
            })?;
            let (reply_id, result) = parse_response(frame)?;
            if reply_id.as_u64() == Some(id) {
                return result.map_err(Error::Rpc);
            }
        }
    }

    fn tick(&self) -> bool {
        std::thread::sleep(POLL);
        true
    }
}

static LAUNCHES: AtomicU64 = AtomicU64::new(0);

/// A launched app. Kills it and removes its directory on drop, then releases the machine lock.
pub struct AppHandle {
    session: Session,
    child: Child,
    dir: PathBuf,
    socket: PathBuf,
    stderr: Arc<Mutex<Vec<String>>>,
    lock: Option<ndshell::lock::MachineLock>,
}

impl AppHandle {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    /// The per-launch directory (`ND_APP_DATA_DIR` lives in `data/` under it).
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The last `n` lines the app wrote to stderr.
    pub fn stderr_tail(&self, n: usize) -> Vec<String> {
        let lines = self.stderr.lock().expect("stderr lines");
        lines[lines.len().saturating_sub(n)..].to_vec()
    }

    /// Whether this launch holds the machine-wide lock.
    pub fn holds_machine_lock(&self) -> bool {
        self.lock.is_some()
    }

    /// Waits until a stderr line contains `needle`.
    pub fn wait_for_stderr(&self, needle: &str, timeout: Duration) -> Result<String> {
        let start = Instant::now();
        loop {
            if let Some(line) = self.stderr.lock().expect("stderr lines").iter().find(|l| l.contains(needle)) {
                return Ok(line.clone());
            }
            if start.elapsed() >= timeout {
                return Err(Error::Launch(format!("\"{needle}\" not on stderr within {timeout:?}")));
            }
            std::thread::sleep(POLL);
        }
    }
}

impl Drop for AppHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
        drop(self.lock.take());
    }
}

impl Deref for AppHandle {
    type Target = Session;
    fn deref(&self) -> &Session {
        &self.session
    }
}

/// Starts the app with `NATIVE_AUTOMATION=1` and connects to its socket.
pub fn launch_app(options: LaunchOptions) -> Result<AppHandle> {
    if !options.bin.exists() {
        return Err(Error::Launch(format!(
            "{} does not exist; build it first (cargo build --workspace --bins)",
            options.bin.display()
        )));
    }
    let lock = if options.machine_lock {
        let purpose = format!("nd-test launch {}", options.bin.display());
        Some(ndshell::lock::acquire_bounded(&purpose).map_err(|e| Error::Launch(format!("real-input lock: {e}")))?)
    } else {
        None
    };
    let n = LAUNCHES.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("nd-test-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("data"))?;
    let socket = dir.join("a.sock");
    let mut command = Command::new(&options.bin);
    command
        .args(&options.args)
        .env("NATIVE_AUTOMATION", "1")
        .env("ND_AUTOMATION_SOCKET", &socket)
        .env("ND_APP_DATA_DIR", dir.join("data"))
        .envs(options.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| Error::Launch(format!("{}: {e}", options.bin.display())))?;
    #[cfg(windows)]
    kill_with_us(&child);
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let pipe = child.stderr.take().expect("piped stderr");
    let sink = stderr.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(std::result::Result::ok) {
            sink.lock().expect("stderr lines").push(line);
        }
    });
    let start = Instant::now();
    let path = loop {
        let found = stderr
            .lock()
            .expect("stderr lines")
            .iter()
            .find_map(|l| l.split_once(READY_MARKER).map(|(_, p)| PathBuf::from(p.trim())));
        if let Some(path) = found {
            break path;
        }
        if let Some(status) = child.try_wait()? {
            let tail = stderr.lock().expect("stderr lines").join("\n");
            let _ = std::fs::remove_dir_all(&dir);
            return Err(Error::Launch(format!("{} exited ({status}) before listening\n{tail}", options.bin.display())));
        }
        if start.elapsed() >= options.ready_timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&dir);
            return Err(Error::Launch(format!("no {READY_MARKER} line within {:?}", options.ready_timeout)));
        }
        std::thread::sleep(POLL);
    };
    let driver = match SocketDriver::connect(&path) {
        Ok(driver) => driver,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
    };
    Ok(AppHandle { session: Session::new(Rc::new(driver)), child, dir, socket: path, stderr, lock })
}

/// Puts `child` in a job object this process holds open with kill-on-close. Windows has no
/// parent-death signal: an [`AppHandle`] a test leaks (or a panic that skips its drop) would
/// otherwise leave the app and the crash handler it starts running, holding its `.exe` so the
/// next build cannot replace it. The job ends them all when this process exits.
#[cfg(windows)]
fn kill_with_us(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };

    /// The job's handle, as an integer so the static is `Sync`; never closed before exit.
    static JOB: OnceLock<Option<isize>> = OnceLock::new();
    let job = JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(None, None).ok()?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            std::ptr::from_ref(&limits).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
        .ok()?;
        Some(job.0 as isize)
    });
    if let Some(job) = *job {
        // A failure (the test runner already put us in a job that forbids nesting) leaves the
        // child as before: killed by its handle's drop.
        let _ = unsafe { AssignProcessToJobObject(HANDLE(job as _), HANDLE(child.as_raw_handle())) };
    }
}

/// Attaches to an app someone else launched, by its socket path (`ND_AUTOMATION_SOCKET`).
pub fn connect_app(socket: &Path) -> Result<Session> {
    Ok(Session::new(Rc::new(SocketDriver::connect(socket)?)))
}
