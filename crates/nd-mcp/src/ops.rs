//! The operations the JS prelude can ask for, run against the live `nd_test::Session`.

use std::path::Path;

use nd_test::{Session, WaitState};
use serde_json::{Value, json};

fn text(op: &Value, key: &str) -> Result<String, String> {
    op.get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("`{key}` must be a string"))
}

fn to_json<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

pub fn dispatch(session: &Session, op: &Value) -> Result<Value, String> {
    let kind = text(op, "op")?;
    let window = op.get("window").and_then(Value::as_u64);
    match kind.as_str() {
        "snapshot" => Ok(Value::String(session.tree(window).map_err(|e| e.to_string())?.snapshot())),
        "tree" => to_json(session.tree(window).map_err(|e| e.to_string())?),
        "key" => session.keyboard().press(&text(op, "keys")?).map_err(|e| e.to_string()),
        "type" => session.keyboard().type_text(&text(op, "text")?).map_err(|e| e.to_string()),
        "rpc" => {
            let method = text(op, "method")?;
            if method == "screenshot" {
                let path = op.pointer("/params/path").and_then(Value::as_str).ok_or("screenshot needs a path")?;
                let window = op.pointer("/params/window").and_then(Value::as_u64);
                return session.screenshot(Path::new(path), window).map_err(|e| e.to_string());
            }
            session.call(&method, op.get("params").cloned().unwrap_or_else(|| json!({}))).map_err(|e| e.to_string())
        }
        "locator" => locator(session, op),
        other => Err(format!("unknown op `{other}`")),
    }
}

fn locator(session: &Session, op: &Value) -> Result<Value, String> {
    let selector = text(op, "selector")?;
    let action = text(op, "action")?;
    let arg = op.get("arg").and_then(Value::as_str).unwrap_or_default();
    let target = session.locator(&selector);
    let err = |e: nd_test::Error| e.to_string();
    let done = |r: Result<(), nd_test::Error>| r.map(|()| Value::Null).map_err(err);
    match action.as_str() {
        "click" => done(target.click()),
        "dblclick" => done(target.dblclick()),
        "right_click" => done(target.right_click()),
        "hover" => done(target.hover()),
        "focus" => done(target.focus()),
        "fill" => done(target.fill(arg)),
        "type" => done(target.type_text(arg)),
        "press" => done(target.press(arg)),
        "scroll_into_view" => done(target.scroll_into_view()),
        "count" => target.count().map(|n| json!(n)).map_err(err),
        "all" => to_json(target.all().map_err(err)?),
        "node" => to_json(target.node().map_err(err)?),
        "text" => target.text().map(Value::String).map_err(err),
        "value" => target.value().map(|v| json!(v)).map_err(err),
        "is_visible" => target.is_visible().map(|v| json!(v)).map_err(err),
        "is_enabled" => target.is_enabled().map(|v| json!(v)).map_err(err),
        "is_checked" => target.is_checked().map(|v| json!(v)).map_err(err),
        "is_focused" => target.is_focused().map(|v| json!(v)).map_err(err),
        "wait_for" => {
            let state = match arg {
                "attached" => WaitState::Attached,
                "detached" => WaitState::Detached,
                "visible" | "" => WaitState::Visible,
                "hidden" => WaitState::Hidden,
                other => return Err(format!("unknown wait state `{other}`")),
            };
            done(target.wait_for(state))
        }
        other => Err(format!("unknown locator action `{other}`")),
    }
}
