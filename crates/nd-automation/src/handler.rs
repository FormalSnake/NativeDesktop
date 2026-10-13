//! One RPC against the app, on the thread that owns it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use nd_core::semantic::{SemanticNode, SemanticTree, SemanticWindow};
use nd_core::{NodeId, Point, WindowId};
use nd_ui::semantics::{ActionError, MouseButton, PointerPhase, SemanticAction, SyntheticInput};
use nd_ui::{App, Keystroke};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::protocol::{RpcError, RpcErrorCode};

fn params<T: for<'de> Deserialize<'de>>(params: &Value) -> Result<T, RpcError> {
    let value = if params.is_null() { json!({}) } else { params.clone() };
    serde_json::from_value(value).map_err(|e| RpcError::invalid_params(e.to_string()))
}

fn action_error(node: Option<NodeId>, e: ActionError) -> RpcError {
    let reference = node.map(|n| n.0);
    match e {
        ActionError::NotActionable(reason) | ActionError::Unsupported(reason) => {
            RpcError::new(RpcErrorCode::NotActionable, "not actionable")
                .with_data(json!({ "ref": reference, "reason": reason }))
        }
        ActionError::InputUnsupported => {
            RpcError::new(RpcErrorCode::InputUnsupported, "input synthesis unsupported on this backend")
        }
        ActionError::WindowGone => RpcError::new(RpcErrorCode::WindowGone, "window closed"),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowParam {
    window: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Target {
    #[serde(rename = "ref")]
    reference: Option<u64>,
    test_id: Option<String>,
    window: Option<u64>,
}

fn scoped(tree: SemanticTree, window: Option<u64>) -> Result<SemanticTree, RpcError> {
    let Some(window) = window else { return Ok(tree) };
    let windows: Vec<SemanticWindow> = tree.windows.into_iter().filter(|w| w.id.0 == window).collect();
    if windows.is_empty() {
        return Err(RpcError::new(RpcErrorCode::WindowGone, "window closed").with_data(json!({ "window": window })));
    }
    Ok(SemanticTree { windows })
}

fn actionable(window: &SemanticWindow, node: &SemanticNode) -> bool {
    window.visible && node.visible && node.enabled
}

/// Nodes carrying `test_id`, best first: actionable ones, then those in the key window, then
/// tree order.
fn ranked(tree: &SemanticTree, test_id: &str) -> Vec<NodeId> {
    let mut matches: Vec<(bool, bool, usize, NodeId)> = tree
        .nodes()
        .enumerate()
        .filter(|(_, (_, n))| n.test_id.as_deref() == Some(test_id))
        .map(|(i, (w, n))| (!actionable(w, n), !w.key, i, n.id))
        .collect();
    matches.sort();
    matches.into_iter().map(|(.., id)| id).collect()
}

/// The node a targeting request names.
fn resolve_target(app: &App, target: &Target) -> Result<NodeId, RpcError> {
    match (target.reference, &target.test_id) {
        (Some(r), None) => {
            let node = NodeId(r);
            if app.window_of(node).is_none() {
                return Err(action_error(Some(node), ActionError::NotActionable("unknown".into())));
            }
            Ok(node)
        }
        (None, Some(test_id)) => {
            let tree = scoped(app.semantic_tree(), target.window)?;
            ranked(&tree, test_id).first().copied().ok_or_else(|| {
                RpcError::new(RpcErrorCode::NotActionable, "not actionable")
                    .with_data(json!({ "ref": null, "testId": test_id, "reason": "unknown" }))
            })
        }
        _ => Err(RpcError::invalid_params("exactly one of ref and testId")),
    }
}

/// The window input goes to: the one named, else the key window, else the first.
fn input_window(app: &App, window: Option<u64>) -> Result<WindowId, RpcError> {
    let tree = app.semantic_tree();
    if let Some(w) = window {
        return tree
            .window(WindowId(w))
            .map(|w| w.id)
            .ok_or_else(|| RpcError::new(RpcErrorCode::WindowGone, "window closed").with_data(json!({ "window": w })));
    }
    let key = tree.windows.iter().find(|w| w.key).or(tree.windows.first());
    key.map(|w| w.id).ok_or_else(|| RpcError::new(RpcErrorCode::WindowGone, "no window open"))
}

fn perform(
    app: &mut App,
    p: &Value,
    action: impl FnOnce(&Value) -> Result<SemanticAction, RpcError>,
) -> Result<Value, RpcError> {
    let target: Target = params(p)?;
    let node = resolve_target(app, &target)?;
    let action = action(p)?;
    app.perform(node, &action).map_err(|e| action_error(Some(node), e))?;
    Ok(json!({ "ref": node.0 }))
}

/// A space-separated keystroke sequence in GPUI's syntax: `cmd-a`, `shift-tab`, `enter`.
pub fn parse_keys(keys: &str) -> Result<Vec<Keystroke>, RpcError> {
    keys.split_whitespace()
        .map(|k| Keystroke::parse(k).map_err(|e| RpcError::invalid_params(format!("keystroke \"{k}\": {e}"))))
        .collect()
}

/// Posts each keystroke through the backend; a backend that cannot synthesize input gets them
/// through nd dispatch directly, which is what its own key handling would call.
fn keys(app: &mut App, window: WindowId, keystrokes: &[Keystroke]) -> Result<Value, RpcError> {
    let mut dispatched = Vec::new();
    for keystroke in keystrokes {
        match app.synthesize(window, &SyntheticInput::Key(keystroke.clone())) {
            Ok(()) => dispatched.push("posted"),
            Err(ActionError::InputUnsupported) => {
                let result = app.dispatch_keystroke(window, keystroke);
                dispatched.push(match result {
                    nd_ui::dispatch::KeyDispatch::Handled => "handled",
                    nd_ui::dispatch::KeyDispatch::Pending => "pending",
                    nd_ui::dispatch::KeyDispatch::Unhandled => "unhandled",
                });
            }
            Err(e) => return Err(action_error(None, e)),
        }
    }
    Ok(json!({ "keys": dispatched }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PointerParams {
    phase: String,
    x: f32,
    y: f32,
    button: Option<String>,
    click_count: Option<u32>,
    window: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DragParams {
    from_ref: Option<u64>,
    to_ref: Option<u64>,
    from_x: Option<f32>,
    from_y: Option<f32>,
    to_x: Option<f32>,
    to_y: Option<f32>,
    steps: Option<u32>,
    window: Option<u64>,
}

fn center_of(tree: &SemanticTree, node: u64) -> Result<(WindowId, Point), RpcError> {
    let (window, n) = tree
        .find(NodeId(node))
        .ok_or_else(|| action_error(Some(NodeId(node)), ActionError::NotActionable("unknown".into())))?;
    let b = n.bounds.ok_or_else(|| action_error(Some(n.id), ActionError::NotActionable("no geometry".into())))?;
    Ok((window.id, Point { x: b.origin.x + b.size.width / 2., y: b.origin.y + b.size.height / 2. }))
}

fn drag_end(
    tree: &SemanticTree,
    node: Option<u64>,
    x: Option<f32>,
    y: Option<f32>,
    which: &str,
) -> Result<(Option<WindowId>, Point), RpcError> {
    match (node, x, y) {
        (Some(n), None, None) => center_of(tree, n).map(|(w, p)| (Some(w), p)),
        (None, Some(x), Some(y)) => Ok((None, Point { x, y })),
        _ => Err(RpcError::invalid_params(format!("{which}: either {which}Ref or {which}X and {which}Y"))),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScreenshotParams {
    path: Option<PathBuf>,
    window: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolveParams {
    test_id: String,
    window: Option<u64>,
    actionable: Option<bool>,
}

/// `waitFor`'s parameters.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitParams {
    pub condition: WaitCondition,
    pub timeout_ms: Option<u64>,
    pub window: Option<u64>,
}

impl WaitParams {
    pub fn parse(p: &Value) -> Result<Self, RpcError> {
        let parsed: Self = params(p)?;
        let c = &parsed.condition;
        let selectors = [c.text_contains.is_some(), c.ref_visible.is_some(), c.test_id.is_some()];
        if selectors.iter().filter(|s| **s).count() != 1 {
            return Err(RpcError::invalid_params("exactly one of textContains, refVisible and testId"));
        }
        if c.test_id.is_none()
            && (c.state.is_some()
                || c.count_at_least.is_some()
                || c.value_equals.is_some()
                || c.value_contains.is_some())
        {
            return Err(RpcError::invalid_params("state, countAtLeast and value refinements need testId"));
        }
        Ok(parsed)
    }

    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms.unwrap_or(2000)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitCondition {
    pub text_contains: Option<String>,
    pub ref_visible: Option<u64>,
    pub test_id: Option<String>,
    /// `present` (default), `gone`, `visible`, `enabled`, `disabled`, `focused`.
    pub state: Option<String>,
    pub count_at_least: Option<usize>,
    pub value_equals: Option<String>,
    pub value_contains: Option<String>,
}

/// One evaluation of a `waitFor` condition on the retained tree. `Some(result)` when it holds.
pub fn wait_check(app: &App, wait: &WaitParams) -> Result<Option<Value>, RpcError> {
    let tree = match scoped(app.semantic_tree(), wait.window) {
        Ok(tree) => tree,
        // A window that closed satisfies nothing but `gone`.
        Err(_) if wait.condition.state.as_deref() == Some("gone") => return Ok(Some(json!({ "matched": 0 }))),
        Err(e) => return Err(e),
    };
    let c = &wait.condition;
    if let Some(text) = &c.text_contains {
        let hit = tree.nodes().find(|(w, n)| w.visible && n.visible && n.name.contains(text.as_str()));
        return Ok(hit.map(|(_, n)| json!({ "ref": n.id.0 })));
    }
    if let Some(r) = c.ref_visible {
        let hit = tree.find(NodeId(r)).is_some_and(|(w, n)| w.visible && n.visible);
        return Ok(hit.then(|| json!({ "ref": r })));
    }
    let test_id = c.test_id.as_deref().unwrap_or_default();
    let state = c.state.as_deref().unwrap_or("present");
    let matches: Vec<(&SemanticWindow, &SemanticNode)> =
        tree.nodes().filter(|(_, n)| n.test_id.as_deref() == Some(test_id)).collect();
    let value_ok = |n: &SemanticNode| {
        let value = n.value.clone().unwrap_or_default();
        c.value_equals.as_ref().is_none_or(|v| value == *v)
            && c.value_contains.as_ref().is_none_or(|v| value.contains(v.as_str()))
    };
    let holds = |(w, n): &(&SemanticWindow, &SemanticNode)| {
        value_ok(n)
            && match state {
                "present" | "gone" => true,
                "visible" => w.visible && n.visible,
                "enabled" => n.enabled,
                "disabled" => !n.enabled,
                "focused" => n.focused,
                _ => false,
            }
    };
    if !["present", "gone", "visible", "enabled", "disabled", "focused"].contains(&state) {
        return Err(RpcError::invalid_params(format!("unknown state \"{state}\"")));
    }
    let count = matches.iter().filter(|m| holds(m)).count();
    let satisfied = if state == "gone" { count == 0 } else { count >= c.count_at_least.unwrap_or(1) };
    Ok(satisfied.then(|| json!({ "matched": count })))
}

static SCREENSHOTS: AtomicU64 = AtomicU64::new(0);

fn screenshot(app: &mut App, p: &Value) -> Result<Value, RpcError> {
    let p: ScreenshotParams = params(p)?;
    let window = input_window(app, p.window)?;
    let capture = app.capture(window).map_err(|e| action_error(None, e))?;
    let path = p.path.unwrap_or_else(|| {
        let dir = std::env::var_os("ND_APP_DATA_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
        let n = SCREENSHOTS.fetch_add(1, Ordering::Relaxed);
        dir.join(format!("nd-screenshot-{}-{n}.png", std::process::id()))
    });
    std::fs::write(&path, &capture.png).map_err(|e| RpcError::internal(format!("{}: {e}", path.display())))?;
    Ok(json!({ "path": path, "width": capture.width, "height": capture.height, "window": window.0 }))
}

/// Runs `method` against `app`. `waitFor` is not handled here: callers poll [`wait_check`].
pub fn handle(app: &mut App, method: &str, p: &Value) -> Result<Value, RpcError> {
    match method {
        "getTree" => {
            let p: WindowParam = params(p)?;
            let tree = scoped(app.semantic_tree(), p.window)?;
            serde_json::to_value(tree).map_err(|e| RpcError::internal(e.to_string()))
        }
        "windows" => {
            let tree = app.semantic_tree();
            Ok(json!(
                tree.windows
                    .iter()
                    .map(|w| json!({
                        "id": w.id.0, "title": w.title, "key": w.key, "main": w.main, "visible": w.visible,
                        "frame": w.frame.map(|f| json!({ "x": f.origin.x, "y": f.origin.y, "width": f.size.width, "height": f.size.height })),
                    }))
                    .collect::<Vec<_>>()
            ))
        }
        "resolve" => {
            let p: ResolveParams = params(p)?;
            let tree = scoped(app.semantic_tree(), p.window)?;
            let mut refs = ranked(&tree, &p.test_id);
            if p.actionable.unwrap_or(true) {
                refs.retain(|r| tree.find(*r).is_some_and(|(w, n)| actionable(w, n)));
            }
            Ok(json!({ "refs": refs.iter().map(|r| r.0).collect::<Vec<_>>() }))
        }
        "click" => perform(app, p, |_| Ok(SemanticAction::Click)),
        "doubleClick" => perform(app, p, |_| Ok(SemanticAction::DoubleClick)),
        "rightClick" => perform(app, p, |_| Ok(SemanticAction::RightClick)),
        "hover" => perform(app, p, |_| Ok(SemanticAction::Hover)),
        "focus" => perform(app, p, |_| Ok(SemanticAction::Focus)),
        "scrollIntoView" => perform(app, p, |_| Ok(SemanticAction::ScrollIntoView)),
        "setValue" => perform(app, p, |p| {
            let value = p.get("value").ok_or_else(|| RpcError::invalid_params("value is required"))?;
            Ok(SemanticAction::SetValue(match value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            }))
        }),
        "type" => perform(app, p, |p| {
            let text =
                p.get("text").and_then(Value::as_str).ok_or_else(|| RpcError::invalid_params("text is required"))?;
            Ok(SemanticAction::Type(text.to_owned()))
        }),
        "scroll" => perform(app, p, |p| {
            let get = |k: &str| p.get(k).and_then(Value::as_f64).unwrap_or(0.) as f32;
            Ok(SemanticAction::Scroll { dx: get("dx"), dy: get("dy") })
        }),
        "keys" => {
            let text =
                p.get("keys").and_then(Value::as_str).ok_or_else(|| RpcError::invalid_params("keys is required"))?;
            let keystrokes = parse_keys(text)?;
            let window = input_window(app, p.get("window").and_then(Value::as_u64))?;
            keys(app, window, &keystrokes)
        }
        "pointer" => {
            let p: PointerParams = params(p)?;
            let phase = match p.phase.as_str() {
                "down" => PointerPhase::Down,
                "up" => PointerPhase::Up,
                "move" => PointerPhase::Move,
                other => return Err(RpcError::invalid_params(format!("phase \"{other}\": want down, up or move"))),
            };
            let button = match p.button.as_deref().unwrap_or("left") {
                "left" => MouseButton::Left,
                "right" => MouseButton::Right,
                "middle" => MouseButton::Middle,
                other => return Err(RpcError::invalid_params(format!("button \"{other}\""))),
            };
            let window = input_window(app, p.window)?;
            let input = SyntheticInput::Pointer {
                phase,
                at: Point { x: p.x, y: p.y },
                button,
                click_count: p.click_count.unwrap_or(if phase == PointerPhase::Move { 0 } else { 1 }),
            };
            app.synthesize(window, &input).map_err(|e| action_error(None, e))?;
            Ok(json!({}))
        }
        "drag" => {
            let p: DragParams = params(p)?;
            let tree = app.semantic_tree();
            let (from_window, from) = drag_end(&tree, p.from_ref, p.from_x, p.from_y, "from")?;
            let (_, to) = drag_end(&tree, p.to_ref, p.to_x, p.to_y, "to")?;
            let window = match from_window {
                Some(w) => w,
                None => input_window(app, p.window)?,
            };
            let input = SyntheticInput::Drag { from, to, steps: p.steps.unwrap_or(12) };
            app.synthesize(window, &input).map_err(|e| action_error(None, e))?;
            Ok(json!({}))
        }
        "screenshot" => screenshot(app, p),
        "waitFor" => Err(RpcError::internal("waitFor is polled by the caller through wait_check")),
        other => Err(RpcError::new(RpcErrorCode::MethodNotFound, format!("method not found: {other}"))),
    }
}
