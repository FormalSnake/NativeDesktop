//! Just enough of the Chrome DevTools Protocol to load a page and take screenshots of it while it
//! runs. `--screenshot` fires once, at load, which on a runner drawing in software is before the
//! first frame is presented.

use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result, bail};
use base64::Engine;
use serde_json::{Value, json};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

pub struct Page {
    chrome: Child,
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    next: u64,
    session: String,
}

impl Page {
    /// Starts `chrome` (already given its flags) and opens `url` in a tab.
    pub fn open(mut chrome: Command, url: &str) -> Result<Page> {
        chrome.arg("--remote-debugging-port=0").arg("about:blank").stdout(Stdio::null()).stderr(Stdio::piped());
        let mut child = chrome.spawn().context("starting Chrome")?;
        let stderr = child.stderr.take().context("Chrome's stderr")?;
        let mut lines = BufReader::new(stderr).lines();
        let endpoint = loop {
            let Some(line) = lines.next() else {
                bail!("Chrome exited before it printed its DevTools address");
            };
            if let Some(address) = line?.split("DevTools listening on ").nth(1) {
                break address.trim().to_owned();
            }
        };
        // Chrome blocks on a full stderr pipe, so keep reading it.
        std::thread::spawn(move || lines.for_each(drop));
        let (socket, _) = tungstenite::connect(&endpoint).with_context(|| format!("connecting to {endpoint}"))?;
        if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
            stream.set_read_timeout(Some(std::time::Duration::from_secs(60)))?;
        }
        let mut page = Page { chrome: child, socket, next: 0, session: String::new() };
        let target = page.call("Target.createTarget", json!({ "url": url }), false)?;
        let target_id = target["targetId"].as_str().context("target id")?.to_owned();
        let attached = page.call("Target.attachToTarget", json!({ "targetId": target_id, "flatten": true }), false)?;
        page.session = attached["sessionId"].as_str().context("session id")?.to_owned();
        Ok(page)
    }

    /// The PNG of the tab's viewport as it is now.
    pub fn screenshot(&mut self) -> Result<Vec<u8>> {
        let shot = self.call("Page.captureScreenshot", json!({ "format": "png" }), true)?;
        let data = shot["data"].as_str().context("screenshot data")?;
        Ok(base64::engine::general_purpose::STANDARD.decode(data)?)
    }

    fn call(&mut self, method: &str, params: Value, in_session: bool) -> Result<Value> {
        self.next += 1;
        let id = self.next;
        let mut message = json!({ "id": id, "method": method, "params": params });
        if in_session {
            message["sessionId"] = json!(self.session);
        }
        self.socket.send(Message::text(message.to_string()))?;
        loop {
            let Message::Text(text) = self.socket.read()? else { continue };
            let reply: Value = serde_json::from_str(text.as_str())?;
            if reply["id"] == id {
                if let Some(error) = reply.get("error") {
                    bail!("{method}: {error}");
                }
                return Ok(reply["result"].clone());
            }
        }
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        let _ = self.chrome.kill();
        let _ = self.chrome.wait();
    }
}
