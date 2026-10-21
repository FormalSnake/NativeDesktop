//! A local HTTP server for the parity and extensions scenes: a few fixed pages, a download, an icon,
//! and `/origin`, which answers the request's `Origin` header.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

pub const INDEX: &str = "<!doctype html><html><head><title>Parity Index</title>\
<link rel=\"icon\" href=\"/icon.png\"></head><body style=\"font:14px -apple-system\">\
<h1>Parity</h1><p id=\"word\">needle in a haystack</p><input id=\"field\">\
<a id=\"blank\" target=\"_blank\" href=\"/second\">second in a new window</a></body></html>";

pub const SECOND: &str = "<!doctype html><html><head><title>Second</title></head><body>Second page</body></html>";

/// A login form for the extensions scene. Its first script records whether the test
/// extension's content script ran before it, at document start.
pub const LOGIN: &str = "<!doctype html><html><head><title>Login</title><script>\
window.ndFirstDocument = document.documentElement.dataset.ndExtension === '1';</script></head>\
<body><form><input id=\"user\"><input id=\"password\" type=\"password\"></form></body></html>";

/// The 0.5.11 app's adblock fixture (nativebrowser `scripts/adblock-drive.ts`), its network
/// cases: each script or image is a real ad-network URL, and the title reports what survived.
pub const ADBLOCK: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>adblock fixture</title>
<script src="https://securepubads.g.doubleclick.net/tag/js/gpt.js"></script>
<script src="https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js"></script>
<script src="https://www.google-analytics.com/analytics.js"></script>
<script src="/js/prebid.js"></script>
</head><body style="margin:0;font:16px system-ui">
<img id="px" src="https://googleads.g.doubleclick.net/pagead/viewthroughconversion/1/?nb=1" width="1" height="1">
<script>
  addEventListener("load", function () {
    setTimeout(function () {
      document.title = "R" + JSON.stringify({
        gptStub: typeof window.googletag === "object",
        adsbygoogle: typeof window.adsbygoogle !== "undefined",
        analytics: typeof window.ga === "function",
        prebid: !!window.__prebid,
        pixel: document.getElementById("px").naturalWidth > 0,
      });
    }, 1500);
  });
</script></body></html>"#;

/// A page a `$csp=script-src 'none'` rule matches: its inline script must not run.
pub const CSP_PROBE: &str = "<!doctype html><html><head><title>csp</title></head><body>\
<script>window.ran = true;</script>csp probe</body></html>";

pub const DOWNLOAD_BODY: &str = "hello download";

/// Serves on an ephemeral port in a thread of its own; returns `http://127.0.0.1:<port>`.
pub fn start() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    let base = format!("http://127.0.0.1:{}", listener.local_addr().expect("local addr").port());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    return;
                }
                let mut origin = "none".to_owned();
                let mut header = String::new();
                while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
                    if let Some((name, value)) = header.split_once(':')
                        && name.eq_ignore_ascii_case("origin")
                    {
                        origin = value.trim().to_owned();
                    }
                    header.clear();
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/");
                if path == "/slow" {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                }
                let (status, kind, extra, body): (&str, &str, &str, Vec<u8>) = match path {
                    "/" => ("200 OK", "text/html; charset=utf-8", "", INDEX.into()),
                    "/adblock" => ("200 OK", "text/html; charset=utf-8", "", ADBLOCK.into()),
                    "/nd-csp-probe" => ("200 OK", "text/html; charset=utf-8", "", CSP_PROBE.into()),
                    "/js/prebid.js" => ("200 OK", "text/javascript", "", b"window.__prebid = 1".to_vec()),
                    "/login" => ("200 OK", "text/html; charset=utf-8", "", LOGIN.into()),
                    "/second" | "/slow" => ("200 OK", "text/html; charset=utf-8", "", SECOND.into()),
                    "/download" => (
                        "200 OK",
                        "application/octet-stream",
                        "Content-Disposition: attachment; filename=\"parity.txt\"\r\n",
                        DOWNLOAD_BODY.into(),
                    ),
                    "/icon.png" => ("200 OK", "image/png", "", Vec::new()),
                    "/origin" => ("200 OK", "text/plain", "Access-Control-Allow-Origin: *\r\n", origin.into_bytes()),
                    _ => ("404 Not Found", "text/plain", "", b"not found".to_vec()),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            });
        }
    });
    base
}
