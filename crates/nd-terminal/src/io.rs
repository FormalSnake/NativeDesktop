//! The byte transport under a session: a child on a pseudo-terminal, or any reader and writer
//! pair (a socket to a remote session). Reading and writing run on their own threads, so a
//! child that floods output or stops reading input never blocks the UI thread; the session
//! drains the inbox on the UI thread when a waker tells it to.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

/// Output buffered past this blocks the reader, which backs the child off through the pty.
const INBOX_LIMIT: usize = 16 << 20;
const READ_CHUNK: usize = 64 << 10;

/// What to run on the pseudo-terminal.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Command {
    /// Program and arguments; empty runs the user's login shell.
    pub argv: Vec<OsString>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(OsString, OsString)>,
}

impl Command {
    /// The user's shell, as a login shell.
    pub fn shell() -> Self {
        Self::default()
    }

    pub fn new(program: impl Into<OsString>) -> Self {
        Self { argv: vec![program.into()], ..Self::default() }
    }

    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.argv.push(arg.into());
        self
    }

    pub fn args<I: IntoIterator<Item = S>, S: Into<OsString>>(mut self, args: I) -> Self {
        self.argv.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
}

/// How the child ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitStatus {
    pub code: u32,
    /// The signal that ended it, by name, when one did.
    pub signal: Option<String>,
}

impl ExitStatus {
    pub fn success(&self) -> bool {
        self.code == 0 && self.signal.is_none()
    }
}

pub(crate) type Waker = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Inbox {
    chunks: VecDeque<Vec<u8>>,
    bytes: usize,
    /// The reader hit end of stream.
    closed: bool,
    exit: Option<ExitStatus>,
}

/// State the I/O threads share with the session.
pub(crate) struct Shared {
    inbox: Mutex<Inbox>,
    room: Condvar,
    wakers: Mutex<Vec<(u64, Waker)>>,
    shutdown: AtomicBool,
}

impl Shared {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inbox: Mutex::default(),
            room: Condvar::new(),
            wakers: Mutex::default(),
            shutdown: AtomicBool::new(false),
        })
    }

    fn wake(&self) {
        let wakers: Vec<Waker> = self.wakers.lock().unwrap().iter().map(|(_, w)| w.clone()).collect();
        for waker in wakers {
            waker();
        }
    }

    pub(crate) fn add_waker(&self, id: u64, waker: Waker) {
        self.wakers.lock().unwrap().push((id, waker));
    }

    pub(crate) fn remove_waker(&self, id: u64) {
        self.wakers.lock().unwrap().retain(|(i, _)| *i != id);
    }

    /// Moves up to `budget` bytes of output to `out`, whole chunks only.
    pub(crate) fn drain(&self, budget: usize, out: &mut Vec<Vec<u8>>) -> Drained {
        let mut inbox = self.inbox.lock().unwrap();
        let mut taken = 0;
        while taken < budget
            && let Some(chunk) = inbox.chunks.pop_front()
        {
            taken += chunk.len();
            out.push(chunk);
        }
        inbox.bytes -= taken;
        let drained = Drained {
            more: !inbox.chunks.is_empty(),
            closed: inbox.closed && inbox.chunks.is_empty(),
            exit: inbox.exit.clone(),
        };
        drop(inbox);
        if taken > 0 {
            self.room.notify_all();
        }
        drained
    }

    pub(crate) fn pending(&self) -> bool {
        !self.inbox.lock().unwrap().chunks.is_empty()
    }
}

pub(crate) struct Drained {
    pub more: bool,
    pub closed: bool,
    pub exit: Option<ExitStatus>,
}

/// The session's end of a transport.
pub(crate) struct Transport {
    pub shared: Arc<Shared>,
    input: mpsc::Sender<Vec<u8>>,
    pty: Option<Box<dyn MasterPty + Send>>,
    killer: Option<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
    pub pid: Option<u32>,
}

impl Transport {
    pub fn spawn(command: &Command, cols: u16, rows: u16) -> io::Result<Self> {
        let system = native_pty_system();
        let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
        let pair = system.openpty(size).map_err(io::Error::other)?;
        let mut builder = if command.argv.is_empty() {
            let mut b = CommandBuilder::new_default_prog();
            if let Some(cwd) = &command.cwd {
                b.cwd(cwd);
            }
            b
        } else {
            let mut b = CommandBuilder::from_argv(command.argv.clone());
            if let Some(cwd) = &command.cwd {
                b.cwd(cwd);
            }
            b
        };
        builder.env("TERM", "xterm-256color");
        builder.env("COLORTERM", "truecolor");
        builder.env("TERM_PROGRAM", "nativedesktop");
        for (key, value) in &command.env {
            builder.env(key, value);
        }
        let mut child = pair.slave.spawn_command(builder).map_err(io::Error::other)?;
        // Ours stays open until the child is gone: when a short-lived child's session ends with
        // no slave left open, macOS can drop the output it wrote before the master reads it.
        // Closing ours after the exit is what lets the master see end of stream.
        let slave = pair.slave;
        let pid = child.process_id();
        let killer = child.clone_killer();
        let reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
        let writer = pair.master.take_writer().map_err(io::Error::other)?;
        let shared = Shared::new();
        let waiter = shared.clone();
        thread::Builder::new().name("nd-terminal-wait".into()).spawn(move || {
            let status = child.wait();
            let status = match status {
                Ok(s) => ExitStatus { code: s.exit_code(), signal: s.signal().map(str::to_owned) },
                Err(_) => ExitStatus { code: 1, signal: None },
            };
            drop(slave);
            waiter.inbox.lock().unwrap().exit = Some(status);
            waiter.wake();
        })?;
        let input = start_io(shared.clone(), reader, writer)?;
        Ok(Self { shared, input, pty: Some(pair.master), killer: Some(killer), pid })
    }

    /// Any byte stream: output read on a thread from `reader`, input written to `writer`.
    pub fn stream(reader: Box<dyn Read + Send>, writer: Box<dyn Write + Send>) -> io::Result<Self> {
        let shared = Shared::new();
        let input = start_io(shared.clone(), reader, writer)?;
        Ok(Self { shared, input, pty: None, killer: None, pid: None })
    }

    /// Where input for the program goes; the writer thread drains it in order.
    pub fn input(&self) -> mpsc::Sender<Vec<u8>> {
        self.input.clone()
    }

    pub fn resize(&self, cols: u16, rows: u16, width_px: u32, height_px: u32) {
        if let Some(pty) = &self.pty {
            let clamp = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
            let _ = pty.resize(PtySize { rows, cols, pixel_width: clamp(width_px), pixel_height: clamp(height_px) });
        }
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        self.shared.room.notify_all();
        if let Some(mut killer) = self.killer.take() {
            let _ = killer.kill();
        }
    }
}

fn start_io(
    shared: Arc<Shared>,
    mut reader: Box<dyn Read + Send>,
    mut writer: Box<dyn Write + Send>,
) -> io::Result<mpsc::Sender<Vec<u8>>> {
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    thread::Builder::new().name("nd-terminal-write".into()).spawn(move || {
        for bytes in rx {
            if writer.write_all(&bytes).and_then(|()| writer.flush()).is_err() {
                break;
            }
        }
    })?;
    thread::Builder::new().name("nd-terminal-read".into()).spawn(move || {
        let mut buf = vec![0u8; READ_CHUNK];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                // Linux reports the child closing its side as EIO.
                Err(_) => break,
            };
            let mut inbox = shared.inbox.lock().unwrap();
            while inbox.bytes >= INBOX_LIMIT && !shared.shutdown.load(Ordering::SeqCst) {
                inbox = shared.room.wait(inbox).unwrap();
            }
            if shared.shutdown.load(Ordering::SeqCst) {
                return;
            }
            let was_empty = inbox.chunks.is_empty();
            inbox.bytes += n;
            inbox.chunks.push_back(buf[..n].to_vec());
            drop(inbox);
            if was_empty {
                shared.wake();
            }
        }
        shared.inbox.lock().unwrap().closed = true;
        shared.wake();
    })?;
    Ok(tx)
}
