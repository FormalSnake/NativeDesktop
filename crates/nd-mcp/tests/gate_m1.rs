//! M1 gate, MCP leg: `snapshot` returns a tree over stdio. Runs against the in-process
//! `examples/hello` stand-in (`--stub`), not a real window, until `nd-automation` is merged.

mod gate_m1 {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

    use serde_json::{Value, json};

    struct Client {
        child: Child,
        stdin: ChildStdin,
        stdout: BufReader<ChildStdout>,
        next_id: u64,
    }

    impl Client {
        fn start() -> Self {
            let mut child = Command::new(env!("CARGO_BIN_EXE_nd-mcp"))
                .arg("--stub")
                .env_remove("ND_AUTOMATION_SOCKET")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .expect("spawn nd-mcp");
            let stdin = child.stdin.take().expect("stdin");
            let stdout = BufReader::new(child.stdout.take().expect("stdout"));
            let mut client = Self { child, stdin, stdout, next_id: 0 };
            client.request(
                "initialize",
                json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "gate", "version": "0" } }),
            );
            client.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
            client
        }

        fn send(&mut self, message: &Value) {
            writeln!(self.stdin, "{message}").expect("write");
            self.stdin.flush().expect("flush");
        }

        fn request(&mut self, method: &str, params: Value) -> Value {
            self.next_id += 1;
            let id = self.next_id;
            self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
            loop {
                let mut line = String::new();
                assert!(self.stdout.read_line(&mut line).expect("read") > 0, "nd-mcp closed stdout");
                let message: Value = serde_json::from_str(&line).expect("json line");
                if message["id"] == id {
                    assert!(message.get("error").is_none(), "{message}");
                    return message["result"].clone();
                }
            }
        }

        fn tool(&mut self, name: &str, arguments: Value) -> String {
            let result = self.request("tools/call", json!({ "name": name, "arguments": arguments }));
            result["content"][0]["text"].as_str().expect("text content").to_string()
        }
    }

    impl Drop for Client {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[test]
    fn mcp_lists_the_three_tools() {
        let mut client = Client::start();
        let tools = client.request("tools/list", json!({}));
        let mut names: Vec<&str> =
            tools["tools"].as_array().expect("tools").iter().filter_map(|t| t["name"].as_str()).collect();
        names.sort();
        assert_eq!(names, ["execute", "reset", "snapshot"]);
    }

    #[test]
    fn mcp_snapshot_returns_the_tree_from_hello() {
        let mut client = Client::start();
        let tree = client.tool("snapshot", json!({}));
        assert!(tree.contains("- window \"Hello\" [ref=e"), "{tree}");
        assert!(tree.contains("button \"Add one\""), "{tree}");
        assert!(tree.contains("text \"0\""), "{tree}");
    }

    #[test]
    fn mcp_execute_clicks_and_reset_clears_state() {
        let mut client = Client::start();
        let out = client.tool("execute", json!({ "code": "state.n = 7; app.click(app.find({ role: 'button' })[0]); app.find({ role: 'text' }).map(n => n.name)" }));
        let out: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(out["result"], json!(["1", "clicks"]), "{out}");

        let out = client.tool("execute", json!({ "code": "state.n" }));
        assert_eq!(serde_json::from_str::<Value>(&out).expect("json")["result"], json!(7));

        let reset = client.tool("reset", json!({}));
        assert!(reset.contains("stub"), "{reset}");
        let out = client.tool("execute", json!({ "code": "[state.n ?? null, app.find({ role: 'text' })[0].name]" }));
        assert_eq!(serde_json::from_str::<Value>(&out).expect("json")["result"], json!([null, "0"]));
    }

    #[test]
    fn mcp_execute_reports_errors_and_timeouts() {
        let mut client = Client::start();
        let out = client.tool("execute", json!({ "code": "app.click(9999)" }));
        assert!(out.contains("no node e9999"), "{out}");
        let out = client.tool("execute", json!({ "code": "while (true) {}", "timeout": 200 }));
        assert!(out.contains("timed out after 200ms"), "{out}");
    }
}
