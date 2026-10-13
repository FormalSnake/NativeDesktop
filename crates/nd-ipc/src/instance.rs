//! Single-instance lock. [`acquire`] either becomes the primary instance, which then receives
//! every later launch as a [`Launch`] on [`Primary::next_launch`], or forwards this process's
//! args and URLs to the running primary and returns [`Instance::Forwarded`], after which the
//! caller exits.
//!
//! Ownership is a file lock (`File::try_lock`, released by the OS when the process dies, so a
//! crash leaves no stale lock) held for the primary's lifetime. The socket is only the
//! forwarding channel, so a stale socket file from a crash is replaced, never trusted.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use interprocess::local_socket::{ListenerOptions, tokio::prelude::*};
use serde::{Deserialize, Serialize};
use tokio::io::BufReader;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::Endpoint;
use crate::framing::{connect, read_line, write_json};

const FORWARD_TIMEOUT: Duration = Duration::from_secs(2);
const ACQUIRE_ATTEMPTS: u32 = 25;
const ACQUIRE_PAUSE: Duration = Duration::from_millis(100);

/// What a launch carried: command line args, the subset of them that are URLs
/// (`myapp://open/1`), and the working directory the launcher ran in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Launch {
    pub args: Vec<String>,
    pub urls: Vec<String>,
    pub cwd: Option<PathBuf>,
}

impl Launch {
    /// This process's args (without argv[0]) and cwd.
    pub fn from_env() -> Self {
        Self::from_args(std::env::args().skip(1))
    }

    pub fn from_args(args: impl IntoIterator<Item = String>) -> Self {
        let args: Vec<String> = args.into_iter().collect();
        let urls = args.iter().filter(|a| is_url(a)).cloned().collect();
        Self { args, urls, cwd: std::env::current_dir().ok() }
    }
}

/// `scheme://rest` with an RFC 3986 scheme. A Windows path (`C:\x`) has no `//`, so it never
/// matches.
fn is_url(arg: &str) -> bool {
    let Some((scheme, rest)) = arg.split_once("://") else { return false };
    !rest.is_empty()
        && scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
}

#[derive(Serialize, Deserialize)]
struct Ack {
    ok: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("instance socket i/o: {0}")]
    Io(#[from] io::Error),
    #[error("a running instance holds the lock but did not answer within {FORWARD_TIMEOUT:?}")]
    Unresponsive,
}

pub enum Instance {
    /// This process owns the app; keep the value alive for as long as it should.
    Primary(Primary),
    /// The launch was handed to the running instance. Exit now.
    Forwarded,
}

pub struct Primary {
    launches: mpsc::Receiver<Launch>,
    task: JoinHandle<()>,
    socket: PathBuf,
    // Held for its lock; closed on drop.
    _lock: File,
}

impl Primary {
    /// The next launch forwarded by a second process. `None` once the accept loop has died.
    pub async fn next_launch(&mut self) -> Option<Launch> {
        self.launches.recv().await
    }

    /// Launches already queued, without waiting. For polling from a UI tick.
    pub fn try_next_launch(&mut self) -> Option<Launch> {
        self.launches.try_recv().ok()
    }
}

impl Drop for Primary {
    fn drop(&mut self) {
        self.task.abort();
        if !cfg!(windows) {
            let _ = fs::remove_file(&self.socket);
        }
    }
}

/// Becomes the primary instance or forwards `launch` to it. Must run inside a tokio runtime.
pub async fn acquire(endpoint: &Endpoint, launch: &Launch) -> Result<Instance, Error> {
    if let Some(dir) = endpoint.lock_path().parent() {
        fs::create_dir_all(dir)?;
    }
    let lock = OpenOptions::new().create(true).truncate(false).write(true).open(endpoint.lock_path())?;
    for _ in 0..ACQUIRE_ATTEMPTS {
        match forward(endpoint, launch).await {
            Ok(()) => return Ok(Instance::Forwarded),
            Err(Error::Io(e)) if is_no_listener(&e) => {}
            Err(e) => return Err(e),
        }
        match lock.try_lock() {
            Ok(()) => return Ok(Instance::Primary(listen(endpoint, lock)?)),
            // A primary holds the lock but is not listening yet (starting up) or is shutting down.
            Err(TryLockError::WouldBlock) => tokio::time::sleep(ACQUIRE_PAUSE).await,
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
    }
    Err(Error::Unresponsive)
}

fn is_no_listener(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused)
}

async fn forward(endpoint: &Endpoint, launch: &Launch) -> Result<(), Error> {
    let exchange = async {
        let stream = connect(endpoint).await?;
        let (read, mut write) = tokio::io::split(stream);
        write_json(&mut write, launch).await?;
        let mut read = BufReader::new(read);
        let mut line = String::new();
        if !read_line(&mut read, &mut line).await? {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        let ack: Ack = serde_json::from_str(&line).map_err(io::Error::other)?;
        if ack.ok { Ok(()) } else { Err(io::Error::other("primary rejected the launch")) }
    };
    match tokio::time::timeout(FORWARD_TIMEOUT, exchange).await {
        Ok(result) => result.map_err(Error::Io),
        Err(_) => Err(Error::Unresponsive),
    }
}

fn listen(endpoint: &Endpoint, lock: File) -> Result<Primary, Error> {
    if !cfg!(windows) {
        // Only reached with the lock held, so any socket file here is a dead process's.
        match fs::remove_file(endpoint.socket_path()) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    let listener = ListenerOptions::new().name(endpoint.name()?).create_tokio()?;
    let (tx, launches) = mpsc::channel(32);
    let task = tokio::spawn(async move {
        loop {
            let stream = match listener.accept().await {
                Ok(stream) => stream,
                Err(e) => {
                    tracing::debug!("instance accept failed: {e}");
                    continue;
                }
            };
            let tx = tx.clone();
            tokio::spawn(async move {
                if let Err(e) = serve_one(stream, tx).await {
                    tracing::debug!("forwarded launch failed: {e}");
                }
            });
        }
    });
    Ok(Primary { launches, task, socket: endpoint.socket_path().to_path_buf(), _lock: lock })
}

async fn serve_one(stream: crate::LocalStream, tx: mpsc::Sender<Launch>) -> io::Result<()> {
    let (read, mut write) = tokio::io::split(stream);
    let mut read = BufReader::new(read);
    let mut line = String::new();
    if !read_line(&mut read, &mut line).await? {
        return Ok(());
    }
    let ok = match serde_json::from_str::<Launch>(&line) {
        Ok(launch) => tx.send(launch).await.is_ok(),
        Err(_) => false,
    };
    write_json(&mut write, &Ack { ok }).await
}
