//! One thread owns the connection to the app, because `nd_test::Session` is `Rc`-based and not
//! `Send`. Everything else sends it closures through [`Bridge`].

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use nd_test::{AppHandle, LaunchOptions, Session, connect_app, launch_app};
use serde_json::Value;

/// What the bridge drives.
#[derive(Clone, Debug)]
pub enum Target {
    /// Attach to a host somebody else launched (`ND_AUTOMATION_SOCKET`).
    Socket(PathBuf),
    /// Launch a binary with `NATIVE_AUTOMATION=1`.
    Launch(LaunchOptions),
}

impl Target {
    fn describe(&self) -> String {
        match self {
            Target::Socket(path) => format!("attached to {}", path.display()),
            Target::Launch(options) => format!("launched {}", options.bin.display()),
        }
    }
}

type Job = Box<dyn FnOnce(&Session) -> Result<Value, String> + Send>;

enum Message {
    Run(Job, mpsc::Sender<Result<Value, String>>),
    Reset(mpsc::Sender<Result<String, String>>),
}

/// Holds the launched app (which kills it on drop) or the attached session.
enum Live {
    Launched(AppHandle),
    Attached(Session),
}

impl Live {
    fn session(&self) -> &Session {
        match self {
            Live::Launched(app) => app,
            Live::Attached(session) => session,
        }
    }
}

fn open(target: &Target) -> Result<Live, String> {
    match target {
        Target::Socket(path) => connect_app(path).map(Live::Attached).map_err(|e| e.to_string()),
        Target::Launch(options) => {
            let app = launch_app(options.clone()).map_err(|e| e.to_string())?;
            wait_for_content(&app);
            Ok(Live::Launched(app))
        }
    }
}

/// The socket answers before the first frame has mounted; an agent's first snapshot should not
/// see an empty window. Gives up quietly after 5 s (an app with no windows is legitimate).
fn wait_for_content(session: &Session) {
    for _ in 0..100 {
        if session.snapshot().is_ok_and(|tree| tree.contains("- ")) {
            return;
        }
        thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Cloneable, `Send` handle to the session thread.
#[derive(Clone)]
pub struct Bridge {
    tx: mpsc::Sender<Message>,
}

impl Bridge {
    pub fn spawn(target: Target) -> Self {
        let (tx, rx) = mpsc::channel::<Message>();
        thread::Builder::new()
            .name("nd-mcp-session".into())
            .spawn(move || {
                let mut live: Option<Live> = None;
                while let Ok(message) = rx.recv() {
                    match message {
                        Message::Run(job, reply) => {
                            let answer = match ensure(&target, &mut live) {
                                Ok(session) => job(session),
                                Err(error) => Err(error),
                            };
                            let _ = reply.send(answer);
                        }
                        Message::Reset(reply) => {
                            live = None;
                            let answer = ensure(&target, &mut live).map(|_| target.describe());
                            let _ = reply.send(answer);
                        }
                    }
                }
            })
            .expect("spawn session thread");
        Self { tx }
    }

    /// Runs `job` against the app on the session thread, launching or attaching first if needed.
    pub fn run(&self, job: impl FnOnce(&Session) -> Result<Value, String> + Send + 'static) -> Result<Value, String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx.send(Message::Run(Box::new(job), reply_tx)).map_err(|_| gone())?;
        reply_rx.recv().map_err(|_| gone())?
    }

    /// Closes the app and starts it again (or reconnects); returns what is being driven.
    pub fn reset(&self) -> Result<String, String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx.send(Message::Reset(reply_tx)).map_err(|_| gone())?;
        reply_rx.recv().map_err(|_| gone())?
    }
}

fn gone() -> String {
    "the session thread is gone".into()
}

fn ensure<'a>(target: &Target, live: &'a mut Option<Live>) -> Result<&'a Session, String> {
    if live.is_none() {
        *live = Some(open(target)?);
    }
    live.as_ref().map(Live::session).ok_or_else(gone)
}
