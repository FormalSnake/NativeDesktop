//! The socket server end to end, with a `TestApp` on its own thread standing in for the main
//! thread: framing, the executor hop, host-side `waitFor` and error codes.

use std::os::unix::net::UnixStream;
use std::sync::mpsc;

use nd_automation::protocol::{parse_response, read_frame, request, write_frame};
use nd_automation::{Job, RpcError, RpcErrorCode};
use nd_ui::test_app::TestApp;
use nd_ui::*;
use serde_json::{Value, json};

struct Counter {
    count: u32,
}

impl Render for Counter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .child(button("add", "Add one").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.count += 1;
                cx.notify();
            })))
            .child(text(format!("count {}", self.count)))
    }
}

/// Starts the server with jobs running on an app thread; returns a connected client.
fn serve(name: &str) -> UnixStream {
    let (tx, rx) = mpsc::channel::<Job>();
    std::thread::spawn(move || {
        // Every measured node is 80 by 20, so text is visible.
        let mut backend = nd_ui::test_backend::TestBackend::new();
        backend.on_measure(|_, _, _| Size { width: 80., height: 20. });
        let mut app = TestApp::with_backend(backend);
        app.open_window(Size { width: 300., height: 200. }, |_, cx| cx.new(|_| Counter { count: 0 }));
        for job in rx {
            job(&mut app);
            app.run_until_idle();
        }
    });
    let tx = std::sync::Mutex::new(tx);
    let path = std::env::temp_dir().join(format!("nd-automation-{name}-{}.sock", std::process::id()));
    nd_automation::start(&path, move |job: Job| tx.lock().unwrap().send(job).unwrap()).unwrap();
    UnixStream::connect(&path).unwrap()
}

fn call(stream: &mut UnixStream, id: u64, method: &str, params: Value) -> Result<Value, RpcError> {
    write_frame(stream, &request(id, method, params)).unwrap();
    let (reply_id, result) = parse_response(read_frame(stream).unwrap().unwrap()).unwrap();
    assert_eq!(reply_id, json!(id));
    result
}

#[test]
fn click_and_wait_over_the_socket() {
    let mut s = serve("click");
    let tree = call(&mut s, 1, "getTree", json!({})).unwrap();
    let button = &tree["windows"][0]["root"]["children"][0]["children"][0];
    assert_eq!((button["role"].as_str(), button["testId"].as_str()), (Some("button"), Some("add")));

    assert!(call(&mut s, 2, "click", json!({ "testId": "add" })).is_ok());
    let waited = call(&mut s, 3, "waitFor", json!({ "condition": { "textContains": "count 1" }, "timeoutMs": 1000 }));
    assert!(waited.unwrap()["ref"].is_u64());

    let by_ref = button["ref"].as_u64().unwrap();
    assert!(call(&mut s, 4, "click", json!({ "ref": by_ref })).is_ok());
    let waited = call(&mut s, 5, "waitFor", json!({ "condition": { "testId": "add", "state": "enabled" } }));
    assert_eq!(waited.unwrap()["matched"], 1);
    let windows = call(&mut s, 6, "windows", Value::Null).unwrap();
    assert_eq!(windows[0]["key"], true);
}

#[test]
fn errors_carry_the_old_codes() {
    let mut s = serve("errors");
    let code = |r: Result<Value, RpcError>| r.unwrap_err().kind();
    assert_eq!(
        code(call(&mut s, 1, "waitFor", json!({ "condition": { "textContains": "never" }, "timeoutMs": 120 }))),
        Some(RpcErrorCode::WaitForTimeout)
    );
    assert_eq!(code(call(&mut s, 2, "click", json!({ "testId": "missing" }))), Some(RpcErrorCode::NotActionable));
    assert_eq!(code(call(&mut s, 3, "click", json!({ "ref": 1, "testId": "add" }))), Some(RpcErrorCode::InvalidParams));
    assert_eq!(code(call(&mut s, 4, "getTree", json!({ "window": 99 }))), Some(RpcErrorCode::WindowGone));
    assert_eq!(
        code(call(&mut s, 5, "pointer", json!({ "phase": "down", "x": 1, "y": 1 }))),
        Some(RpcErrorCode::InputUnsupported)
    );
    assert_eq!(code(call(&mut s, 6, "bogus", json!({}))), Some(RpcErrorCode::MethodNotFound));
    assert_eq!(
        code(call(&mut s, 7, "waitFor", json!({ "condition": { "textContains": "a", "testId": "b" } }))),
        Some(RpcErrorCode::InvalidParams)
    );
}
