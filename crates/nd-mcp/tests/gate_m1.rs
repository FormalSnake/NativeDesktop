//! M1 gate, MCP leg: `snapshot` returns the semantic tree of a running `hello` over stdio.
//! The `real_` tests launch the app (a real window; build it first: `cargo build --workspace --bins`).

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
                .args(["--example", "hello"])
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
    fn real_mcp_snapshot_returns_the_tree_from_hello() {
        let mut client = Client::start();
        let tree = client.tool("snapshot", json!({}));
        assert!(tree.contains("window \"Hello\""), "{tree}");
        assert!(tree.contains("button \"Add one\"") && tree.contains("[testid=add]"), "{tree}");
        assert!(tree.contains("text \"0\""), "{tree}");
    }

    #[test]
    fn real_mcp_execute_clicks_and_reset_clears_state() {
        let mut client = Client::start();
        let out = client.tool(
            "execute",
            json!({ "code": "state.n = 7; app.getByTestId('add').click(); app.getByText('1').waitFor('visible'); app.getByText('1').count()" }),
        );
        let out: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(out["result"], json!(1), "{out}");

        let out = client.tool("execute", json!({ "code": "state.n" }));
        assert_eq!(serde_json::from_str::<Value>(&out).expect("json")["result"], json!(7));

        let reset = client.tool("reset", json!({}));
        assert!(reset.contains("launched"), "{reset}");
        let out = client.tool(
            "execute",
            json!({ "code": "app.getByText('0').waitFor('visible'); [state.n ?? null, app.getByText('0').count()]" }),
        );
        assert_eq!(serde_json::from_str::<Value>(&out).expect("json")["result"], json!([null, 1]));
    }

    #[test]
    fn real_mcp_execute_reports_errors_and_timeouts() {
        let mut client = Client::start();
        let out = client.tool("execute", json!({ "code": "app.getByTestId('nope').click()" }));
        assert!(out.contains("\"error\""), "{out}");
        let out = client.tool("execute", json!({ "code": "while (true) {}", "timeout": 200 }));
        assert!(out.contains("timed out after 200ms"), "{out}");
    }
}
