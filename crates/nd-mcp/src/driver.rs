//! The seam between the bridge and a running app.
//!
//! `nd-automation` and `nd-test` own the real implementation (the automation socket, locators
//! and the actionability ladder). This crate is written against the small [`Driver`] trait so the
//! bridge works before they merge: [`crate::stub`] implements it over an in-process tree, and the
//! socket driver replaces `Launcher::launch` for `ND_AUTOMATION_SOCKET` once it exists.

use std::fmt;

use serde::Serialize;

/// One node of the semantic tree (ARCHITECTURE.md 10.1). `id` is the wire ref shown as `[ref=eN]`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Node {
    pub id: u32,
    pub role: String,
    pub name: Option<String>,
    pub value: Option<String>,
    pub test_id: Option<String>,
    pub enabled: bool,
    pub focused: bool,
    pub children: Vec<Node>,
}

impl Node {
    /// Depth-first, self included.
    pub fn walk<'a>(&'a self, out: &mut Vec<&'a Node>) {
        out.push(self);
        for child in &self.children {
            child.walk(out);
        }
    }

    pub fn find(&self, id: u32) -> Option<&Node> {
        let mut all = Vec::new();
        self.walk(&mut all);
        all.into_iter().find(|n| n.id == id)
    }
}

#[derive(Debug)]
pub struct DriverError(pub String);

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DriverError {}

impl From<String> for DriverError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

impl From<&str> for DriverError {
    fn from(message: &str) -> Self {
        Self(message.to_string())
    }
}

/// A live app. Not `Send`: it lives on the session thread for its whole life.
pub trait Driver {
    fn tree(&mut self) -> Result<Node, DriverError>;
    /// Semantic click on the node with this ref.
    fn click(&mut self, node: u32) -> Result<(), DriverError>;
    fn alive(&mut self) -> bool;
}

/// Starts or attaches to an app. Runs on the session thread, so the driver it returns need not
/// be `Send`.
pub trait Launcher: Send + 'static {
    fn launch(&mut self) -> Result<Box<dyn Driver>, DriverError>;
    /// What the user is driving, for `reset` replies.
    fn describe(&self) -> String;
}

/// `ND_AUTOMATION_SOCKET`: attach to a host somebody else launched.
pub struct SocketLauncher {
    pub socket: String,
}

impl Launcher for SocketLauncher {
    fn launch(&mut self) -> Result<Box<dyn Driver>, DriverError> {
        Err(DriverError(format!(
            "attaching to {} needs nd-automation, which is not part of this build yet",
            self.socket
        )))
    }

    fn describe(&self) -> String {
        format!("socket {}", self.socket)
    }
}

impl Launcher for Box<dyn Launcher> {
    fn launch(&mut self) -> Result<Box<dyn Driver>, DriverError> {
        (**self).launch()
    }

    fn describe(&self) -> String {
        (**self).describe()
    }
}
