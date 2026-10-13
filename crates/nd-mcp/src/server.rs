//! The three MCP tools. Same contract as the old bridge: `execute`, `snapshot`, `reset`.

use std::time::Duration;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{ServerHandler, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::json;

use crate::js::{DEFAULT_TIMEOUT, JsEngine};
use crate::ops;
use crate::session::Bridge;

#[derive(Clone)]
pub struct NdMcp {
    session: Bridge,
    js: JsEngine,
    tool_router: ToolRouter<Self>,
}

impl NdMcp {
    pub fn new(session: Bridge) -> Self {
        let js = JsEngine::spawn(session.clone());
        Self { session, js, tool_router: Self::tool_router() }
    }

    pub fn execute_blocking(&self, code: &str, timeout: Option<u64>) -> serde_json::Value {
        let timeout = timeout.map_or(DEFAULT_TIMEOUT, Duration::from_millis);
        match self.js.execute(code, timeout) {
            Ok(done) => json!({ "result": done.result, "logs": done.logs }),
            Err(failed) => {
                json!({ "error": failed.message, "logs": failed.logs, "hint": "call reset if the app is wedged" })
            }
        }
    }

    pub fn snapshot_blocking(&self, window: Option<u64>) -> Result<String, String> {
        let op = json!({ "op": "snapshot", "window": window });
        let tree = self.session.run(move |session| ops::dispatch(session, &op))?;
        Ok(tree.as_str().unwrap_or_default().to_string())
    }

    pub fn reset_blocking(&self) -> serde_json::Value {
        self.js.reset();
        match self.session.reset() {
            Ok(driving) => json!({ "driving": driving }),
            Err(error) => json!({ "error": error.to_string() }),
        }
    }
}

fn text(value: &serde_json::Value) -> CallToolResult {
    let is_error = value.get("error").is_some();
    let content = vec![ContentBlock::text(serde_json::to_string_pretty(value).unwrap_or_default())];
    if is_error { CallToolResult::error(content) } else { CallToolResult::success(content) }
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ExecuteParams {
    /// JavaScript with `app`, `state` and `console` in scope. Prefer one line, `;` between statements,
    /// and call execute again instead of writing a long script. The value of the last expression is
    /// returned as JSON. `app.snapshot()`, `app.tree()`, `app.getByRole(role, {name})`, `app.getByTestId(id)`,
    /// `app.locator(selector)` (then `.click()`, `.fill(v)`, `.text()`, `.count()`, `.waitFor()`) and
    /// `app.keyboard.press("cmd-a")` are synchronous; `state` survives between calls.
    code: String,
    /// Milliseconds before the snippet is abandoned. Defaults to 30000.
    timeout: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SnapshotParams {
    /// Window id from `app.windows()`; absent means every window.
    window: Option<u64>,
}

#[tool_router]
impl NdMcp {
    #[tool(
        description = "Control the user's NativeDesktop app with JavaScript snippets. Read the tree first (app.snapshot()), act with a locator such as app.getByTestId('add').click(), then read it again to check the change."
    )]
    async fn execute(&self, Parameters(params): Parameters<ExecuteParams>) -> CallToolResult {
        let this = self.clone();
        let value = tokio::task::spawn_blocking(move || this.execute_blocking(&params.code, params.timeout))
            .await
            .unwrap_or_else(|e| json!({ "error": format!("execute panicked: {e}") }));
        text(&value)
    }

    #[tool(
        description = "Compact accessibility tree of the app: one line per node, indented by depth, with the [ref=eN] every action targets. Read this before acting, the way you would read a page."
    )]
    async fn snapshot(&self, Parameters(params): Parameters<SnapshotParams>) -> CallToolResult {
        let this = self.clone();
        let result = tokio::task::spawn_blocking(move || this.snapshot_blocking(params.window))
            .await
            .unwrap_or_else(|e| Err(format!("snapshot panicked: {e}")));
        match result {
            Ok(tree) => CallToolResult::success(vec![ContentBlock::text(tree)]),
            Err(error) => text(&json!({ "error": error, "hint": "call reset" })),
        }
    }

    #[tool(
        description = "Close the app and launch it again, clearing `state`. Use it after a crash, a wedged dialog, or to start a scenario from a known screen. Attaching to a running host reconnects instead of respawning."
    )]
    async fn reset(&self) -> CallToolResult {
        let this = self.clone();
        let value = tokio::task::spawn_blocking(move || this.reset_blocking())
            .await
            .unwrap_or_else(|e| json!({ "error": format!("reset panicked: {e}") }));
        text(&value)
    }
}

#[tool_handler(router = self.tool_router, name = "nativedesktop", instructions = "Drive a NativeDesktop app: snapshot to read the tree, execute to act, reset to start over.")]
impl ServerHandler for NdMcp {}
