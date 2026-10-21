//! A local HTTP server for the parity and extensions scenes: a few fixed pages, a download and an icon.

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
                // Drain the headers.
                let mut header = String::new();
                while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
                    header.clear();
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/");
                if path == "/slow" {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                }
                let (status, kind, extra, body): (&str, &str, &str, Vec<u8>) = match path {
                    "/" => ("200 OK", "text/html; charset=utf-8", "", INDEX.into()),
                    "/login" => ("200 OK", "text/html; charset=utf-8", "", LOGIN.into()),
                    "/second" | "/slow" => ("200 OK", "text/html; charset=utf-8", "", SECOND.into()),
                    "/download" => (
                        "200 OK",
                        "application/octet-stream",
                        "Content-Disposition: attachment; filename=\"parity.txt\"\r\n",
                        DOWNLOAD_BODY.into(),
                    ),
                    "/icon.png" => ("200 OK", "image/png", "", Vec::new()),
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
