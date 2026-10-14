//! `ND_AUTOMATION_DIALOG_SCRIPT`: scripted answers for app-level dialogs, so automated runs
//! never block on real UI. The variable holds JSON, or `@/path/to.json`, mapping a method to a
//! FIFO of answers in the zig branch's shapes:
//!
//! ```json
//! { "dialog.openFile": [{ "paths": ["/tmp/a.txt"] }, { "paths": [] }],
//!   "dialog.saveFile": [{ "path": "/tmp/out.txt" }, { "path": null }],
//!   "dialog.showMessage": [{ "button": 1 }] }
//! ```
//!
//! An empty `paths` or a null `path` is a cancel. A method the script does not mention shows
//! the real dialog; one whose queue ran dry fails loudly instead of falling through to it.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Deserialize;

use crate::{Error, Result};

pub const ENV: &str = "ND_AUTOMATION_DIALOG_SCRIPT";

#[derive(Deserialize)]
struct OpenAnswer {
    paths: Vec<PathBuf>,
}

#[derive(Deserialize)]
struct SaveAnswer {
    path: Option<PathBuf>,
}

#[derive(Deserialize)]
struct MessageAnswer {
    button: usize,
}

/// Parsed answers, shared by every clone.
#[derive(Clone, Debug, Default)]
pub struct DialogScript(Arc<Mutex<HashMap<String, VecDeque<serde_json::Value>>>>);

impl DialogScript {
    pub fn parse(json: &str) -> Result<Self> {
        let map: HashMap<String, VecDeque<serde_json::Value>> =
            serde_json::from_str(json).map_err(|e| Error::Invalid(format!("{ENV}: {e}")))?;
        Ok(Self(Arc::new(Mutex::new(map))))
    }

    /// The script in [`ENV`], if set. A script that does not parse is reported on stderr and
    /// ignored, which shows real dialogs.
    pub fn from_env() -> Option<Self> {
        let raw = std::env::var(ENV).ok().filter(|v| !v.is_empty())?;
        let json = match raw.strip_prefix('@') {
            Some(path) => match std::fs::read_to_string(path) {
                Ok(json) => json,
                Err(e) => {
                    eprintln!("nd-system: {ENV}: {path}: {e}");
                    return None;
                }
            },
            None => raw,
        };
        Self::parse(&json).map_err(|e| eprintln!("nd-system: {e}")).ok()
    }

    fn take<T: for<'de> Deserialize<'de>>(&self, method: &str) -> Option<Result<T>> {
        let mut map = self.0.lock().expect("dialog script");
        let queue = map.get_mut(method)?;
        Some(match queue.pop_front() {
            Some(value) => serde_json::from_value(value).map_err(|e| Error::Invalid(format!("{ENV} {method}: {e}"))),
            None => Err(Error::Invalid(format!("{ENV}: no answers left for {method}"))),
        })
    }

    pub(crate) fn open(&self) -> Option<Result<Option<Vec<PathBuf>>>> {
        self.take::<OpenAnswer>("dialog.openFile").map(|a| a.map(|a| Some(a.paths).filter(|p| !p.is_empty())))
    }

    pub(crate) fn save(&self) -> Option<Result<Option<PathBuf>>> {
        self.take::<SaveAnswer>("dialog.saveFile").map(|a| a.map(|a| a.path))
    }

    pub(crate) fn message(&self) -> Option<Result<usize>> {
        self.take::<MessageAnswer>("dialog.showMessage").map(|a| a.map(|a| a.button))
    }
}
