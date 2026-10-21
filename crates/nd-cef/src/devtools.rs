//! The DevTools protocol session each page keeps with its own browser: script evaluation in
//! named worlds, user scripts, `postMessage` bindings and the user agent. No front-end is
//! involved; messages go through `send_dev_tools_message` and come back to the observer.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use cef::*;
use serde_json::{Value, json};

use super::page;

type Reply = Box<dyn FnOnce(Result<Value, String>)>;

/// The binding pages post through; `window.webkit.messageHandlers.<name>.postMessage` calls it.
pub(crate) const BINDING: &str = "__ndPost";

#[derive(Default)]
pub(crate) struct Devtools {
    next: Cell<i32>,
    pending: RefCell<HashMap<i32, Reply>>,
    registration: RefCell<Option<Registration>>,
    host: RefCell<Option<BrowserHost>>,
    /// Execution contexts by id: (frame id, world name, is the frame's page world).
    contexts: RefCell<HashMap<i64, (String, String, bool)>>,
    main_frame: RefCell<String>,
    /// Bindings added, by world ("" for the page's).
    bound: RefCell<Vec<String>>,
    page: Cell<u64>,
    /// Lookups waiting for the main frame's id.
    waiting: RefCell<Vec<Box<dyn FnOnce()>>>,
}

impl Devtools {
    pub(crate) fn attach(&self, id: u64, host: &BrowserHost) {
        let mut observer = NdObserver::new(id);
        *self.registration.borrow_mut() = host.add_dev_tools_message_observer(Some(&mut observer));
        *self.host.borrow_mut() = Some(host.clone());
        self.page.set(id);
        self.call("Runtime.enable", json!({}), |_| {});
        self.call("Page.enable", json!({}), |_| {});
        self.call("Page.getFrameTree", json!({}), move |r| {
            if let (Ok(tree), Some(page)) = (r, page::get(id))
                && let Some(frame) = tree["frameTree"]["frame"]["id"].as_str()
            {
                page.devtools.set_main_frame(frame);
            }
        });
    }

    pub(crate) fn detach(&self) {
        self.registration.borrow_mut().take();
        self.host.borrow_mut().take();
        for (_, reply) in self.pending.borrow_mut().drain() {
            reply(Err("the page closed".into()));
        }
    }

    /// Sends `method` and hands its result (or error message) to `reply`.
    pub(crate) fn call(&self, method: &str, params: Value, reply: impl FnOnce(Result<Value, String>) + 'static) {
        let Some(host) = self.host.borrow().clone() else {
            reply(Err("the page has no browser yet".into()));
            return;
        };
        let id = self.next.get() + 1;
        self.next.set(id);
        self.pending.borrow_mut().insert(id, Box::new(reply));
        let message = json!({ "id": id, "method": method, "params": params }).to_string();
        if host.send_dev_tools_message(Some(message.as_bytes())) != 1
            && let Some(reply) = self.pending.borrow_mut().remove(&id)
        {
            reply(Err(format!("{method} was refused")));
        }
    }

    pub(crate) fn result(&self, id: i32, success: bool, body: &[u8]) {
        let Some(reply) = self.pending.borrow_mut().remove(&id) else { return };
        let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        if success {
            reply(Ok(value));
        } else {
            reply(Err(value["message"].as_str().map(str::to_owned).unwrap_or_else(|| value.to_string())));
        }
    }

    /// Protocol events; returns a binding call as (world, payload).
    pub(crate) fn event(&self, method: &str, params: &Value) -> Option<(String, String)> {
        match method {
            "Runtime.executionContextCreated" => {
                let c = &params["context"];
                let (Some(id), Some(frame)) = (c["id"].as_i64(), c["auxData"]["frameId"].as_str()) else { return None };
                let name = c["name"].as_str().unwrap_or_default().to_owned();
                let default = c["auxData"]["isDefault"].as_bool().unwrap_or(false);
                self.contexts.borrow_mut().insert(id, (frame.to_owned(), name, default));
            }
            "Runtime.executionContextDestroyed" => {
                if let Some(id) = params["executionContextId"].as_i64() {
                    self.contexts.borrow_mut().remove(&id);
                }
            }
            "Runtime.executionContextsCleared" => self.contexts.borrow_mut().clear(),
            "Page.frameNavigated" => {
                let frame = &params["frame"];
                if frame["parentId"].is_null()
                    && let Some(id) = frame["id"].as_str()
                {
                    self.set_main_frame(id);
                }
            }
            "Runtime.bindingCalled" if params["name"].as_str() == Some(BINDING) => {
                let context = params["executionContextId"].as_i64().unwrap_or_default();
                let world = self
                    .contexts
                    .borrow()
                    .get(&context)
                    .map(|(_, name, default)| if *default { String::new() } else { name.clone() })
                    .unwrap_or_default();
                return Some((world, params["payload"].as_str().unwrap_or_default().to_owned()));
            }
            _ => {}
        }
        None
    }

    fn set_main_frame(&self, id: &str) {
        *self.main_frame.borrow_mut() = id.to_owned();
        let waiting = std::mem::take(&mut *self.waiting.borrow_mut());
        for f in waiting {
            f();
        }
    }

    /// The main frame's context for `world`, created when the page has none yet.
    fn context_for(&self, world: &str, then: impl FnOnce(Result<Option<i64>, String>) + 'static) {
        if world.is_empty() {
            then(Ok(None));
            return;
        }
        if self.main_frame.borrow().is_empty() {
            let (id, world) = (self.page.get(), world.to_owned());
            self.waiting.borrow_mut().push(Box::new(move || match page::get(id) {
                Some(p) => p.devtools.context_for(&world, then),
                None => then(Err("the page closed".into())),
            }));
            return;
        }
        let main = self.main_frame.borrow().clone();
        let found = self
            .contexts
            .borrow()
            .iter()
            .find(|(_, (frame, name, default))| *frame == main && name == world && !default)
            .map(|(id, _)| *id);
        if let Some(id) = found {
            then(Ok(Some(id)));
            return;
        }
        self.call("Page.createIsolatedWorld", json!({ "frameId": main, "worldName": world }), move |r| {
            then(r.map(|v| v["executionContextId"].as_i64()));
        });
    }

    /// Evaluates the framework's own `code` and answers with its completion value: a thrown
    /// exception is an error, a promise is not awaited. It runs without a user gesture, so
    /// nothing the framework runs hands the page user activation (popups, fullscreen,
    /// picture-in-picture) it did not get from the user.
    pub(crate) fn evaluate(&self, code: &str, world: Option<&str>, done: impl FnOnce(Result<Value, String>) + 'static) {
        self.run(code, world, false, done);
    }

    /// [`Self::evaluate`] for the app's `execute_javascript`: the app acts for the user (a
    /// shortcut, a menu item), so the code runs with a user gesture, as on WebKit, and may open
    /// windows, enter fullscreen or picture-in-picture.
    pub(crate) fn evaluate_as_user(
        &self,
        code: &str,
        world: Option<&str>,
        done: impl FnOnce(Result<Value, String>) + 'static,
    ) {
        self.run(code, world, true, done);
    }

    fn run(&self, code: &str, world: Option<&str>, gesture: bool, done: impl FnOnce(Result<Value, String>) + 'static) {
        let code = code.to_owned();
        let id = self.page.get();
        self.context_for(world.unwrap_or_default(), move |context| {
            let Some(page) = page::get(id) else {
                done(Err("the page closed".into()));
                return;
            };
            let mut params =
                json!({ "expression": code, "returnByValue": true, "awaitPromise": false, "userGesture": gesture });
            match context {
                Ok(Some(id)) => params["contextId"] = json!(id),
                Ok(None) => {}
                Err(e) => {
                    done(Err(e));
                    return;
                }
            }
            page.devtools.call("Runtime.evaluate", params, move |r| {
                done(r.and_then(|v| {
                    if let Some(ex) = v.get("exceptionDetails") {
                        let message = ex["exception"]["description"]
                            .as_str()
                            .or(ex["text"].as_str())
                            .unwrap_or("the script threw")
                            .to_owned();
                        return Err(message);
                    }
                    Ok(v["result"]["value"].clone())
                }))
            });
        });
    }

    /// Registers `source` for every new document (in `world` when given) and answers with the
    /// identifier that removes it.
    pub(crate) fn add_script(&self, source: &str, world: Option<&str>, done: impl FnOnce(Option<String>) + 'static) {
        let mut params = json!({ "source": source });
        if let Some(world) = world.filter(|w| !w.is_empty()) {
            params["worldName"] = json!(world);
        }
        self.call("Page.addScriptToEvaluateOnNewDocument", params, move |r| {
            done(r.ok().and_then(|v| v["identifier"].as_str().map(str::to_owned)));
        });
    }

    pub(crate) fn remove_script(&self, identifier: &str) {
        self.call("Page.removeScriptToEvaluateOnNewDocument", json!({ "identifier": identifier }), |_| {});
    }

    /// Exposes [`BINDING`] to `world` (the page's when empty), once per world.
    pub(crate) fn bind(&self, world: &str) {
        if self.bound.borrow().iter().any(|w| w == world) {
            return;
        }
        self.bound.borrow_mut().push(world.to_owned());
        let mut params = json!({ "name": BINDING });
        if !world.is_empty() {
            params["executionContextName"] = json!(world);
        }
        self.call("Runtime.addBinding", params, |_| {});
    }
}

/// The page-side shim behind `window.webkit.messageHandlers.<name>.postMessage(body)`.
pub(crate) fn handler_shim(name: &str) -> String {
    let name = serde_json::to_string(name).unwrap_or_else(|_| "\"\"".into());
    format!(
        "(function(){{var n={name};var w=window.webkit=window.webkit||{{}};var h=w.messageHandlers=w.messageHandlers||{{}};\
         h[n]={{postMessage:function(b){{{BINDING}(JSON.stringify({{name:n,body:b===undefined?null:b}}));}}}};}})();"
    )
}

pub(crate) fn remove_shim(name: &str) -> String {
    let name = serde_json::to_string(name).unwrap_or_else(|_| "\"\"".into());
    format!("(function(){{var w=window.webkit;if(w&&w.messageHandlers)delete w.messageHandlers[{name}];}})();")
}

/// A user script's source for every document CDP runs it in: only the top frame unless
/// `all_frames`, and after parsing for [`nd_webview::Injection::End`].
pub(crate) fn script_source(script: &nd_webview::UserScript) -> String {
    let mut source = script.guarded_source();
    if script.injection == nd_webview::Injection::End {
        source = format!(
            "(function(){{var r=function(){{\n{source}\n}};if(document.readyState==='loading')\
             document.addEventListener('DOMContentLoaded',r,{{once:true}});else r();}})();"
        );
    }
    if !script.all_frames {
        source = format!("if(window.top===window){{\n{source}\n}}");
    }
    source
}

wrap_dev_tools_message_observer! {
    struct NdObserver {
        id: u64,
    }

    impl DevToolsMessageObserver {
        fn on_dev_tools_method_result(&self, _browser: Option<&mut Browser>, message_id: i32, success: i32, result: Option<&[u8]>) {
            if let Some(page) = page::get(self.id) {
                page.devtools.result(message_id, success != 0, result.unwrap_or_default());
            }
        }

        fn on_dev_tools_event(&self, _browser: Option<&mut Browser>, method: Option<&CefString>, params: Option<&[u8]>) {
            let Some(page) = page::get(self.id) else { return };
            let method = method.map(|m| m.to_string()).unwrap_or_default();
            let params: Value = params.and_then(|p| serde_json::from_slice(p).ok()).unwrap_or(Value::Null);
            if let Some((world, payload)) = page.devtools.event(&method, &params) {
                page.script_message(world, &payload);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nd_webview::UserScript;

    #[test]
    fn end_scripts_wait_for_parsing_and_stay_in_the_top_frame() {
        let s = script_source(&UserScript::new("a", "go()"));
        assert!(s.starts_with("if(window.top===window){"), "{s}");
        assert!(s.contains("DOMContentLoaded"), "{s}");
        let all = script_source(&UserScript::new("a", "go()").at_start().all_frames());
        assert_eq!(all, "go()");
    }

    #[test]
    fn shims_post_through_the_binding() {
        let s = handler_shim("bridge");
        assert!(s.contains(r#"var n="bridge""#), "{s}");
        assert!(s.contains(BINDING), "{s}");
    }
}
