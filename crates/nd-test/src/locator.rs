//! Lazy locators. A locator holds a session, selector parts and an optional window scope;
//! nothing resolves until an action or reader runs, and each run reads a fresh tree, so a
//! locator kept across re-renders still finds the right node.
//!
//! The actionability ladder before a single-node action, in order: exactly one match (more is a
//! strict-mode error at once), visible, enabled, real size, and the same bounds on two reads in
//! a row so a moving node is never acted on. A node that went stale between resolving and
//! acting (`notActionable` from the app) goes round the loop again until the timeout.

use std::time::{Duration, Instant};

use nd_core::semantic::{SemanticNode, SemanticTree};
use nd_core::{Rect, WindowId};
use serde_json::{Value, json};

use crate::matcher::{near_misses, roots, select};
use crate::selector::{self, Part, TextMatch};
use crate::session::Session;
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundingBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl From<Rect> for BoundingBox {
    fn from(r: Rect) -> Self {
        Self { x: r.origin.x, y: r.origin.y, width: r.size.width, height: r.size.height }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitState {
    Attached,
    Detached,
    Visible,
    Hidden,
}

#[derive(Clone)]
pub struct Locator {
    session: Session,
    parts: std::result::Result<Vec<Part>, String>,
    window: Option<u64>,
    timeout: Option<Duration>,
}

/// One read of the tree and what the locator matched in it.
pub(crate) struct Resolved {
    pub tree: SemanticTree,
    pub matches: Vec<SemanticNode>,
}

impl Resolved {
    fn window_visible(&self, node: &SemanticNode) -> bool {
        self.tree.find(node.id).is_some_and(|(w, _)| w.visible)
    }

    pub fn visible(&self, node: &SemanticNode) -> bool {
        node.visible && self.window_visible(node)
    }
}

fn describe(node: &SemanticNode) -> String {
    let mut line = format!("{} {:?} [ref=e{}]", node.role, node.name, node.id.0);
    if let Some(id) = &node.test_id {
        line.push_str(&format!(" [testid={id}]"));
    }
    line
}

impl Locator {
    pub(crate) fn new(session: Session, parts: std::result::Result<Vec<Part>, Error>, window: Option<u64>) -> Self {
        Self { session, parts: parts.map_err(|e| e.to_string()), window, timeout: None }
    }

    fn derive(&self, f: impl FnOnce(&mut Vec<Part>)) -> Self {
        let mut next = self.clone();
        if let Ok(parts) = next.parts.as_mut() {
            f(parts);
        }
        next
    }

    fn push(&self, part: Part) -> Self {
        self.derive(|parts| parts.push(part))
    }

    /// The selector this locator prints as in errors.
    pub fn selector(&self) -> String {
        match &self.parts {
            Ok(parts) => selector::format(parts),
            Err(e) => e.clone(),
        }
    }

    pub fn parts(&self) -> Option<&[Part]> {
        self.parts.as_deref().ok()
    }

    /// This locator with its own timeout for actions, readers' waits and `expect`.
    pub fn with_timeout(&self, timeout: Duration) -> Self {
        Self { timeout: Some(timeout), ..self.clone() }
    }

    pub(crate) fn timeout(&self) -> Duration {
        self.timeout.unwrap_or(self.session.timeout)
    }

    pub(crate) fn session(&self) -> &Session {
        &self.session
    }

    // Chaining.

    pub fn locator(&self, selector: &str) -> Self {
        match selector::parse(selector) {
            Ok(more) => self.derive(|parts| parts.extend(more)),
            Err(e) => Self { parts: Err(e.to_string()), ..self.clone() },
        }
    }

    pub fn get_by_test_id(&self, test_id: &str) -> Self {
        self.push(Part::TestId(test_id.to_owned()))
    }

    pub fn get_by_role(&self, role: nd_core::semantic::Role) -> Self {
        self.push(Part::Role { role: role.as_str().to_owned(), name: None, checked: None, disabled: None })
    }

    pub fn get_by_text(&self, text: &str) -> Self {
        self.push(Part::Text(TextMatch::Substring(text.to_owned())))
    }

    pub fn get_by_label(&self, text: &str) -> Self {
        self.push(Part::Label(TextMatch::Substring(text.to_owned())))
    }

    pub fn get_by_placeholder(&self, text: &str) -> Self {
        self.push(Part::Placeholder(TextMatch::Substring(text.to_owned())))
    }

    fn refine_role(&self, f: impl FnOnce(&mut Option<TextMatch>, &mut Option<bool>, &mut Option<bool>)) -> Self {
        self.derive(|parts| {
            if let Some(Part::Role { name, checked, disabled, .. }) = parts.last_mut() {
                f(name, checked, disabled);
            }
        })
    }

    /// On a role locator: the accessible name contains `name` (case-insensitive).
    pub fn name(&self, name: &str) -> Self {
        self.refine_role(|n, _, _| *n = Some(TextMatch::Substring(name.to_owned())))
    }

    /// On a role locator: `[checked]` / `[checked=false]`.
    pub fn checked(&self, checked: bool) -> Self {
        self.refine_role(|_, c, _| *c = Some(checked))
    }

    /// On a role locator: `[disabled]` / `[disabled=false]`.
    pub fn disabled(&self, disabled: bool) -> Self {
        self.refine_role(|_, _, d| *d = Some(disabled))
    }

    /// Makes the last text part (a role's name, `text=`, `label=`, `placeholder=`) match exactly.
    pub fn exact(&self) -> Self {
        let exact = |t: &mut TextMatch| {
            if let TextMatch::Substring(s) = t {
                *t = TextMatch::Exact(std::mem::take(s));
            }
        };
        self.derive(|parts| match parts.last_mut() {
            Some(Part::Role { name: Some(t), .. })
            | Some(Part::Text(t))
            | Some(Part::Label(t))
            | Some(Part::Placeholder(t))
            | Some(Part::HasText(t)) => exact(t),
            _ => {}
        })
    }

    pub fn filter_has_text(&self, text: &str) -> Self {
        self.push(Part::HasText(TextMatch::Substring(text.to_owned())))
    }

    pub fn has(&self, inner: &Locator) -> Self {
        self.push(Part::Has(inner.parts.clone().unwrap_or_default()))
    }

    pub fn has_not(&self, inner: &Locator) -> Self {
        self.push(Part::HasNot(inner.parts.clone().unwrap_or_default()))
    }

    pub fn and(&self, other: &Locator) -> Self {
        self.push(Part::And(other.parts.clone().unwrap_or_default()))
    }

    pub fn first(&self) -> Self {
        self.push(Part::Nth(0))
    }

    pub fn last(&self) -> Self {
        self.push(Part::Nth(-1))
    }

    pub fn nth(&self, index: i64) -> Self {
        self.push(Part::Nth(index))
    }

    // Resolution.

    /// One read, no waiting.
    pub(crate) fn resolve(&self) -> Result<Resolved> {
        let parts = self.parts.as_ref().map_err(|e| Error::Selector(e.clone()))?;
        let tree = match self.session.tree(self.window) {
            Ok(tree) => tree,
            // The scoped window closed: nothing matches.
            Err(Error::Rpc(e)) if e.kind() == Some(nd_automation::RpcErrorCode::WindowGone) => SemanticTree::default(),
            Err(e) => return Err(e),
        };
        let matches = select(&roots(&tree, self.window), parts).into_iter().cloned().collect();
        Ok(Resolved { tree, matches })
    }

    /// The nodes matched now.
    pub fn all(&self) -> Result<Vec<SemanticNode>> {
        Ok(self.resolve()?.matches)
    }

    pub fn count(&self) -> Result<usize> {
        Ok(self.resolve()?.matches.len())
    }

    fn timeout_error(&self, start: Instant, reason: String, tree: &SemanticTree) -> Error {
        let want = self.parts.as_deref().ok().and_then(selector::intended_name).unwrap_or_default();
        Error::Timeout {
            selector: self.selector(),
            waited_ms: start.elapsed().as_millis(),
            reason,
            candidates: near_misses(tree, &want, 5),
        }
    }

    fn strict(&self, matches: &[SemanticNode]) -> Error {
        Error::Strict {
            selector: self.selector(),
            count: matches.len(),
            matches: matches.iter().map(describe).collect(),
        }
    }

    /// Polls until exactly one node matches and, when `actionable`, passes the ladder. Returns
    /// that node.
    pub(crate) fn wait_one(&self, actionable: bool) -> Result<SemanticNode> {
        let start = Instant::now();
        let deadline = start + self.timeout();
        let mut previous: Option<(u64, Option<Rect>)> = None;
        let mut settled = false;
        loop {
            let resolved = self.resolve()?;
            let reason = match resolved.matches.as_slice() {
                [] => {
                    previous = None;
                    "no node matches".to_owned()
                }
                [node] => match self.ladder(&resolved, node, actionable) {
                    None if !actionable => return Ok(node.clone()),
                    None => {
                        // Stable means the same bounds on the next read; the first read rereads
                        // at once, a node still moving waits a tick.
                        let now = Some((node.id.0, node.bounds));
                        if previous == now {
                            return Ok(node.clone());
                        }
                        let first = previous.is_none();
                        previous = now;
                        if first {
                            continue;
                        }
                        format!("{} is still moving", describe(node))
                    }
                    Some(reason) => {
                        previous = None;
                        reason
                    }
                },
                many => return Err(self.strict(many)),
            };
            if Instant::now() >= deadline || settled {
                return Err(self.timeout_error(start, reason, &resolved.tree));
            }
            // An idle in-process app cannot change by waiting: one more read after it settles.
            settled = !self.session.tick();
        }
    }

    /// Why `node` cannot be acted on, or `None`.
    fn ladder(&self, resolved: &Resolved, node: &SemanticNode, actionable: bool) -> Option<String> {
        if !actionable {
            return None;
        }
        if !resolved.visible(node) {
            return Some(format!("{} is not visible", describe(node)));
        }
        if !node.enabled {
            return Some(format!("{} is disabled", describe(node)));
        }
        match node.bounds {
            Some(b) if b.size.width > 0. && b.size.height > 0. => None,
            Some(b) => Some(format!("{} has zero size ({}x{})", describe(node), b.size.width, b.size.height)),
            None => Some(format!("{} has no geometry", describe(node))),
        }
    }

    /// Runs `method` on the one actionable node, retrying when the node went stale.
    fn act(&self, method: &str, extra: Value, actionable: bool) -> Result<SemanticNode> {
        let start = Instant::now();
        loop {
            let node = self.wait_one(actionable)?;
            let mut params = json!({ "ref": node.id.0 });
            if let (Value::Object(p), Value::Object(e)) = (&mut params, &extra) {
                p.extend(e.clone());
            }
            match self.session.call(method, params) {
                Ok(_) => return Ok(node),
                Err(e) if e.is_stale() && start.elapsed() < self.timeout() && self.session.tick() => continue,
                Err(e) => return Err(e),
            }
        }
    }

    // Actions.

    pub fn click(&self) -> Result<()> {
        self.act("click", json!({}), true).map(drop)
    }

    pub fn dblclick(&self) -> Result<()> {
        self.act("doubleClick", json!({}), true).map(drop)
    }

    pub fn right_click(&self) -> Result<()> {
        self.act("rightClick", json!({}), true).map(drop)
    }

    pub fn hover(&self) -> Result<()> {
        self.act("hover", json!({}), true).map(drop)
    }

    pub fn focus(&self) -> Result<()> {
        self.act("focus", json!({}), true).map(drop)
    }

    /// Replaces the control's value.
    pub fn fill(&self, value: &str) -> Result<()> {
        self.act("setValue", json!({ "value": value }), true).map(drop)
    }

    /// Appends text, as typing into the control would.
    pub fn type_text(&self, text: &str) -> Result<()> {
        self.act("type", json!({ "text": text }), true).map(drop)
    }

    /// Focuses the node, then presses `keys` (GPUI syntax, space separated).
    pub fn press(&self, keys: &str) -> Result<()> {
        let node = self.act("focus", json!({}), true)?;
        let window = self.session.tree(None)?.find(node.id).map(|(w, _)| w.id);
        self.session.call("keys", json!({ "keys": keys, "window": window.map(|w: WindowId| w.0) })).map(drop)
    }

    pub fn scroll(&self, dx: f32, dy: f32) -> Result<()> {
        self.act("scroll", json!({ "dx": dx, "dy": dy }), true).map(drop)
    }

    /// Waives visibility: the point is reaching a node that is off screen.
    pub fn scroll_into_view(&self) -> Result<()> {
        self.act("scrollIntoView", json!({}), false).map(drop)
    }

    // Readers. Each waits for exactly one match, without the actionability ladder.

    pub fn node(&self) -> Result<SemanticNode> {
        self.wait_one(false)
    }

    /// The accessible name: a text run's text, a button's title.
    pub fn text(&self) -> Result<String> {
        Ok(self.node()?.name)
    }

    pub fn value(&self) -> Result<Option<String>> {
        Ok(self.node()?.value)
    }

    pub fn is_visible(&self) -> Result<bool> {
        let resolved = self.resolve()?;
        match resolved.matches.as_slice() {
            [] => Ok(false),
            [node] => Ok(resolved.visible(node)),
            many => Err(self.strict(many)),
        }
    }

    pub fn is_enabled(&self) -> Result<bool> {
        Ok(self.node()?.enabled)
    }

    pub fn is_checked(&self) -> Result<bool> {
        Ok(self.node()?.checked == Some(true))
    }

    pub fn is_focused(&self) -> Result<bool> {
        Ok(self.node()?.focused)
    }

    pub fn bounding_box(&self) -> Result<Option<BoundingBox>> {
        Ok(self.node()?.bounds.map(BoundingBox::from))
    }

    /// Waits for the match set to reach `state`.
    pub fn wait_for(&self, state: WaitState) -> Result<()> {
        let start = Instant::now();
        let deadline = start + self.timeout();
        let mut settled = false;
        loop {
            let resolved = self.resolve()?;
            let visible = resolved.matches.iter().filter(|n| resolved.visible(n)).count();
            let done = match state {
                WaitState::Attached => !resolved.matches.is_empty(),
                WaitState::Detached => resolved.matches.is_empty(),
                WaitState::Visible => visible > 0,
                WaitState::Hidden => visible == 0,
            };
            if done {
                return Ok(());
            }
            if Instant::now() >= deadline || settled {
                return Err(self.timeout_error(start, format!("never reached {state:?}"), &resolved.tree));
            }
            settled = !self.session.tick();
        }
    }
}

impl std::fmt::Display for Locator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.selector())
    }
}

impl std::fmt::Debug for Locator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Locator({})", self.selector())
    }
}
