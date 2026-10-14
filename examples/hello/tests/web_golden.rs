//! Tier 2 golden of the plain web look (ARCHITECTURE.md 12): `hello` built with
//! `nd build --target web`, served locally and drawn by headless Chrome, light and dark, compared
//! with `tests/goldens/web-*.png`. It needs Chrome and a wasm build, so it runs only with
//! `ND_WEB_GOLDEN=1` (`just web-test`). `ND_BLESS=1` re-records; a failure leaves
//! `web-*.diff.png` beside the golden. `ND_CHROME` points at another Chrome.

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const SIZE: (u32, u32) = (640, 480);
/// A channel further apart than this counts the pixel as changed.
const CHANNEL_TOLERANCE: u8 = 24;
/// Share of changed pixels a run may have, for antialiasing that differs between GPU paths.
const CHANGED_TOLERANCE: f64 = 0.002;

fn chrome() -> PathBuf {
    if let Some(path) = std::env::var_os("ND_CHROME") {
        return path.into();
    }
    let mac = Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
    if mac.exists() {
        return mac.into();
    }
    ["google-chrome", "chromium"]
        .iter()
        .map(PathBuf::from)
        .find(|p| Command::new(p).arg("--version").output().is_ok())
        .expect("no Chrome: set ND_CHROME")
}

/// Serves `dir` on a free local port until the process exits.
fn serve(dir: PathBuf) -> u16 {
    let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("bind"));
    let port = server.server_addr().to_ip().expect("ip address").port();
    std::thread::spawn(move || {
        for request in server.incoming_requests() {
            let relative = request.url().trim_start_matches('/').split('?').next().unwrap_or_default().to_owned();
            let path = dir.join(if relative.is_empty() { "index.html" } else { &relative });
            let response = match std::fs::read(&path) {
                Ok(bytes) => {
                    let kind = nd_cli::web::content_type(&path);
                    let header = tiny_http::Header::from_bytes("Content-Type", kind).expect("header");
                    tiny_http::Response::from_data(bytes).with_header(header).boxed()
                }
                Err(_) => tiny_http::Response::empty(404).boxed(),
            };
            let _ = request.respond(response);
        }
    });
    port
}

fn decode(path: &Path) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())));
    let mut reader = decoder.read_info().expect("png header");
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("png frame");
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        other => panic!("{}: unexpected colour type {other:?}", path.display()),
    };
    (info.width, info.height, rgba)
}

fn encode(path: &Path, (width, height): (u32, u32), rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("create png");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().expect("png header").write_image_data(rgba).expect("png data");
}

#[test]
fn hello_web_golden() {
    if std::env::var_os("ND_WEB_GOLDEN").is_none_or(|v| v != "1") {
        eprintln!("hello_web_golden: skipped, set ND_WEB_GOLDEN=1 (needs Chrome and a wasm build)");
        return;
    }
    let project = nd_cli::web::WebProject::load(Some("hello")).expect("hello package");
    let work = tempfile::tempdir().expect("temp dir");
    let bundle = work.path().join("bundle");
    nd_cli::web::build_web(&project, &bundle, false).expect("nd build --target web");
    let port = serve(bundle);
    let goldens = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens");
    std::fs::create_dir_all(&goldens).expect("goldens dir");
    // docs:begin bless
    let bless = std::env::var_os("ND_BLESS").is_some_and(|v| v == "1");
    // docs:end bless
    let mut failures = Vec::new();
    // Blink's `PreferredColorScheme`: 0 dark, 1 light.
    for (scheme, setting) in [("light", 1), ("dark", 0)] {
        let shot = work.path().join(format!("{scheme}.png"));
        let status = Command::new(chrome())
            .args([
                "--headless=new",
                "--hide-scrollbars",
                "--force-device-scale-factor=1",
                "--enable-unsafe-webgpu",
                &format!("--window-size={},{}", SIZE.0, SIZE.1),
                &format!("--blink-settings=preferredColorScheme={setting}"),
                "--virtual-time-budget=10000",
                &format!("--user-data-dir={}", work.path().join(format!("profile-{scheme}")).display()),
                &format!("--screenshot={}", shot.display()),
                &format!("http://127.0.0.1:{port}/"),
            ])
            .output()
            .expect("run Chrome")
            .status;
        assert!(status.success(), "Chrome exited with {status}");
        let golden = goldens.join(format!("web-{scheme}.png"));
        let (width, height, actual) = decode(&shot);
        if bless || !golden.exists() {
            encode(&golden, (width, height), &actual);
            eprintln!("hello_web_golden: recorded {}", golden.display());
            continue;
        }
        let (gw, gh, expected) = decode(&golden);
        assert_eq!((width, height), (gw, gh), "{scheme}: size changed");
        let mut diff = vec![0u8; actual.len()];
        let mut changed = 0usize;
        for (i, (a, e)) in actual.chunks(4).zip(expected.chunks(4)).enumerate() {
            let off = a.iter().zip(e).any(|(a, e)| a.abs_diff(*e) > CHANNEL_TOLERANCE);
            changed += usize::from(off);
            let pixel = if off { [255, 0, 255, 255] } else { [e[0] / 3, e[1] / 3, e[2] / 3, 255] };
            diff[i * 4..i * 4 + 4].copy_from_slice(&pixel);
        }
        let share = changed as f64 / (width * height) as f64;
        if share > CHANGED_TOLERANCE {
            let diff_path = goldens.join(format!("web-{scheme}.diff.png"));
            encode(&diff_path, (width, height), &diff);
            failures.push(format!("{scheme}: {:.2}% of pixels changed, see {}", share * 100., diff_path.display()));
        }
    }
    assert!(failures.is_empty(), "web golden drifted (ND_BLESS=1 re-records):\n{}", failures.join("\n"));
}
