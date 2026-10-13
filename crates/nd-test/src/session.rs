//! A connection to one app, whatever the tier, and the locators, keyboard and mouse built on it.

use std::rc::Rc;
use std::time::Duration;

use nd_core::semantic::{Role, SemanticTree};
use serde_json::{Value, json};

use crate::locator::Locator;
use crate::selector::{Part, TextMatch};
use crate::{Error, Result};

/// How requests reach an app. Tier 1 runs them in-process, tier 3 sends them over the socket.
pub trait Driver {
    /// Runs one automation RPC (the `nd-automation` vocabulary).
    fn call(&self, method: &str, params: Value) -> Result<Value>;
    /// Lets the app make progress before the next poll. Returns false when waiting cannot change
    /// anything (an in-process app that is idle), so polling stops instead of spinning.
    fn tick(&self) -> bool;
}

/// A handle on one app: locators, keyboard, mouse and the raw RPCs. Cheap to clone.
#[derive(Clone)]
pub struct Session {
    pub(crate) driver: Rc<dyn Driver>,
    pub(crate) timeout: Duration,
}

pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

impl Session {
    pub fn new(driver: Rc<dyn Driver>) -> Self {
        Self { driver, timeout: DEFAULT_TIMEOUT }
    }

    /// The same app with another default timeout for waiting actions and assertions.
    pub fn with_timeout(&self, timeout: Duration) -> Self {
        Self { driver: self.driver.clone(), timeout }
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// A raw RPC, for methods the locator API does not wrap.
    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.driver.call(method, params)
    }

    pub fn tick(&self) -> bool {
        self.driver.tick()
    }

    /// The semantic tree of every window, or of one.
    pub fn tree(&self, window: Option<u64>) -> Result<SemanticTree> {
        let params = window.map_or_else(|| json!({}), |w| json!({ "window": w }));
        let value = self.call("getTree", params)?;
        serde_json::from_value(value).map_err(|e| Error::Io(std::io::Error::other(format!("getTree reply: {e}"))))
    }

    /// The compact one-line-per-node tree the MCP `snapshot` tool shows.
    pub fn snapshot(&self) -> Result<String> {
        Ok(self.tree(None)?.snapshot())
    }

    /// `[{id, title, key, main, visible, frame}]`.
    pub fn windows(&self) -> Result<Value> {
        self.call("windows", json!({}))
    }

    /// Writes a PNG of the window (the key window when `None`) to `path`.
    pub fn screenshot(&self, path: &std::path::Path, window: Option<u64>) -> Result<Value> {
        self.call("screenshot", json!({ "path": path, "window": window }))
    }

    fn locate(&self, parts: Vec<Part>) -> Locator {
        Locator::new(self.clone(), Ok(parts), None)
    }

    /// A selector string in the old grammar: `role=button[name="Save"] >> nth=1`.
    pub fn locator(&self, selector: &str) -> Locator {
        Locator::new(self.clone(), crate::selector::parse(selector), None)
    }

    pub fn get_by_test_id(&self, test_id: &str) -> Locator {
        self.locate(vec![Part::TestId(test_id.to_owned())])
    }

    /// Refine with [`Locator::name`] / [`Locator::exact`]: `get_by_role(Role::Button).name("Save")`.
    pub fn get_by_role(&self, role: Role) -> Locator {
        self.locate(vec![Part::Role { role: role.as_str().to_owned(), name: None, checked: None, disabled: None }])
    }

    /// Nodes whose own text contains `text` (case-insensitive); [`Locator::exact`] for equality.
    pub fn get_by_text(&self, text: &str) -> Locator {
        self.locate(vec![Part::Text(TextMatch::Substring(text.to_owned()))])
    }

    pub fn get_by_label(&self, text: &str) -> Locator {
        self.locate(vec![Part::Label(TextMatch::Substring(text.to_owned()))])
    }

    pub fn get_by_placeholder(&self, text: &str) -> Locator {
        self.locate(vec![Part::Placeholder(TextMatch::Substring(text.to_owned()))])
    }

    /// Locators scoped to one window, by its title or its index in window order.
    pub fn window(&self, title_or_index: WindowRef<'_>) -> Result<Locator> {
        let tree = self.tree(None)?;
        let window = match title_or_index {
            WindowRef::Title(title) => tree.windows.iter().find(|w| w.title == title),
            WindowRef::Index(i) => tree.windows.get(i),
        }
        .ok_or_else(|| Error::Launch(format!("no window {title_or_index:?}")))?;
        Ok(Locator::new(self.clone(), Ok(Vec::new()), Some(window.id.0)))
    }

    pub fn keyboard(&self) -> Keyboard {
        Keyboard { session: self.clone() }
    }

    pub fn mouse(&self) -> Mouse {
        Mouse { session: self.clone(), at: std::cell::Cell::new((0., 0.)) }
    }

    /// Blocks until the host-side `waitFor` condition holds (see `nd_automation::WaitParams`).
    pub fn wait_for(&self, condition: Value, timeout: Duration) -> Result<Value> {
        self.call("waitFor", json!({ "condition": condition, "timeoutMs": timeout.as_millis() as u64 }))
    }
}

#[derive(Clone, Copy, Debug)]
pub enum WindowRef<'a> {
    Title(&'a str),
    Index(usize),
}

/// Keystrokes into the key window, in GPUI's syntax (`cmd-a`, `shift-tab`, `enter`). Posted as
/// native key events where the backend can, else dispatched through nd's keymap.
pub struct Keyboard {
    session: Session,
}

impl Keyboard {
    /// One or more space-separated keystrokes.
    pub fn press(&self, keys: &str) -> Result<Value> {
        self.session.call("keys", json!({ "keys": keys }))
    }

    /// Types `text` one key at a time.
    pub fn type_text(&self, text: &str) -> Result<Value> {
        let keys: Vec<String> = text
            .chars()
            .map(|c| match c {
                ' ' => "space".to_owned(),
                '\n' => "enter".to_owned(),
                '\t' => "tab".to_owned(),
                c if c.is_uppercase() => format!("shift-{}", c.to_lowercase()),
                c => c.to_string(),
            })
            .collect();
        self.press(&keys.join(" "))
    }
}

/// Pointer events in the key window's content coordinates, posted into the app's own queue.
pub struct Mouse {
    session: Session,
    at: std::cell::Cell<(f32, f32)>,
}

impl Mouse {
    fn pointer(&self, phase: &str, x: f32, y: f32, click_count: u32) -> Result<Value> {
        self.at.set((x, y));
        self.session.call("pointer", json!({ "phase": phase, "x": x, "y": y, "clickCount": click_count }))
    }

    pub fn move_to(&self, x: f32, y: f32) -> Result<Value> {
        self.pointer("move", x, y, 0)
    }

    pub fn down(&self) -> Result<Value> {
        let (x, y) = self.at.get();
        self.pointer("down", x, y, 1)
    }

    pub fn up(&self) -> Result<Value> {
        let (x, y) = self.at.get();
        self.pointer("up", x, y, 1)
    }

    pub fn click(&self, x: f32, y: f32) -> Result<Value> {
        self.pointer("down", x, y, 1)?;
        self.pointer("up", x, y, 1)
    }

    /// Press, move and release as one batch (AppKit tracking loops need the whole gesture).
    pub fn drag(&self, from: (f32, f32), to: (f32, f32)) -> Result<Value> {
        self.at.set(to);
        self.session.call("drag", json!({ "fromX": from.0, "fromY": from.1, "toX": to.0, "toY": to.1 }))
    }
}
