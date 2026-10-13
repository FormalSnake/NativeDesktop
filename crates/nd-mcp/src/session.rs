//! One thread owns the driver, because a driver is not `Send` (the stub holds `Rc`s and the real
//! one holds a socket). Everything else talks to it through [`Session`].

use std::sync::mpsc;
use std::thread;

use crate::driver::{Driver, DriverError, Launcher, Node};

enum Request {
    Tree,
    Click(u32),
    /// Drop the current app and launch a fresh one.
    Reset,
}

enum Response {
    Tree(Node),
    Done,
    Reset(String),
}

type Message = (Request, mpsc::Sender<Result<Response, DriverError>>);

/// Cloneable, `Send` handle to the session thread.
#[derive(Clone)]
pub struct Session {
    tx: mpsc::Sender<Message>,
}

impl Session {
    pub fn spawn(mut launcher: impl Launcher) -> Self {
        let (tx, rx) = mpsc::channel::<Message>();
        thread::Builder::new()
            .name("nd-mcp-session".into())
            .spawn(move || {
                let mut app: Option<Box<dyn Driver>> = None;
                while let Ok((request, reply)) = rx.recv() {
                    let _ = reply.send(handle(&mut launcher, &mut app, request));
                }
            })
            .expect("spawn session thread");
        Self { tx }
    }

    fn call(&self, request: Request) -> Result<Response, DriverError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx.send((request, reply_tx)).map_err(|_| DriverError::from("the session thread is gone"))?;
        reply_rx.recv().map_err(|_| DriverError::from("the session thread is gone"))?
    }

    pub fn tree(&self) -> Result<Node, DriverError> {
        match self.call(Request::Tree)? {
            Response::Tree(node) => Ok(node),
            _ => Err("unexpected reply".into()),
        }
    }

    pub fn click(&self, node: u32) -> Result<(), DriverError> {
        self.call(Request::Click(node)).map(|_| ())
    }

    /// Relaunches; returns a description of what is now being driven.
    pub fn reset(&self) -> Result<String, DriverError> {
        match self.call(Request::Reset)? {
            Response::Reset(description) => Ok(description),
            _ => Err("unexpected reply".into()),
        }
    }
}

fn ensure<'a>(
    launcher: &mut dyn Launcher,
    app: &'a mut Option<Box<dyn Driver>>,
) -> Result<&'a mut Box<dyn Driver>, DriverError> {
    if app.as_mut().is_some_and(|a| !a.alive()) {
        *app = None;
    }
    if app.is_none() {
        *app = Some(launcher.launch()?);
    }
    app.as_mut().ok_or_else(|| DriverError::from("no app"))
}

fn handle(
    launcher: &mut dyn Launcher,
    app: &mut Option<Box<dyn Driver>>,
    request: Request,
) -> Result<Response, DriverError> {
    match request {
        Request::Tree => ensure(launcher, app)?.tree().map(Response::Tree),
        Request::Click(node) => ensure(launcher, app)?.click(node).map(|()| Response::Done),
        Request::Reset => {
            *app = None;
            ensure(launcher, app)?;
            Ok(Response::Reset(launcher.describe()))
        }
    }
}
