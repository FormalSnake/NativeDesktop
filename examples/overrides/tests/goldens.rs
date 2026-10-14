//! Tier 2 goldens of the macOS look, light and dark: the showcase window in this process on
//! the real AppKit backend, captured as the window server composites it (`screencapture -l`,
//! so glass, title bar and traffic lights are in it) and compared with
//! `tests/goldens/macos-<scheme>@<scale>x.png`. Needs Screen Recording for the process that
//! runs the test (`ndshell grant` on CI).
//!
//! `ND_UPDATE_GOLDENS=1` rewrites the goldens. A mismatch writes `<golden>.actual.png` beside
//! the golden. Prints `ND_OVERRIDES_GOLDENS_OK`.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let name = "gate_m3::overrides_goldens";
    if args.iter().any(|a| a == "--list") {
        if !args.iter().any(|a| a == "--ignored") && cfg!(target_os = "macos") {
            println!("{name}: test");
        }
        return;
    }
    let filters: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with('-')).collect();
    if args.iter().any(|a| a == "--ignored")
        || !(filters.is_empty() || filters.iter().any(|f| name.contains(f.as_str())))
    {
        return;
    }
    #[cfg(target_os = "macos")]
    goldens::run();
}

#[cfg(target_os = "macos")]
mod goldens {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use nd::theme::Appearance;

    /// Per channel, out of 255: antialiasing and subpixel positioning drift by a few levels.
    const CHANNEL_TOLERANCE: i16 = 24;
    /// Share of pixels allowed past the channel tolerance.
    const PIXEL_TOLERANCE: f64 = 0.004;

    fn decode(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut decoder = png::Decoder::new(png_bytes);
        decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
        let mut reader = decoder.read_info().expect("png header");
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).expect("png frame");
        buf.truncate(info.buffer_size());
        let channels = info.color_type.samples();
        // Normalise to RGBA.
        let rgba = match channels {
            4 => buf,
            2 => buf.chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
            _ => panic!("unexpected png layout {:?}", info.color_type),
        };
        (info.width, info.height, rgba)
    }

    /// Fraction of pixels that differ past the channel tolerance; `None` when sizes differ.
    fn difference(a: &[u8], b: &[u8]) -> f64 {
        let off = a
            .chunks(4)
            .zip(b.chunks(4))
            .filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| (*x as i16 - *y as i16).abs() > CHANNEL_TOLERANCE))
            .count();
        off as f64 / (a.len() / 4).max(1) as f64
    }

    fn golden(scheme: &str, scale: u32) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/goldens/macos-{scheme}@{scale}x.png"))
    }

    fn settle(host: &nd::appkit::AppHost) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            host.platform().pump(Duration::from_millis(30), Duration::from_millis(300));
            let busy = host.update(|app| {
                app.frame();
                app.needs_frame()
            });
            if !busy || Instant::now() > deadline {
                break;
            }
        }
        // A few more turns so deferred work (toolbar items, traffic lights) lands.
        host.platform().pump(Duration::from_millis(100), Duration::from_millis(400));
    }

    fn is_key(host: &nd::appkit::AppHost, window: &nd::appkit::NdWindow) -> bool {
        window.isKeyWindow() && host.platform().app().isActive()
    }

    fn make_key(host: &nd::appkit::AppHost, window: &nd::appkit::NdWindow) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !is_key(host, window) && Instant::now() < deadline {
            host.platform().show(window);
            host.platform().pump(Duration::from_millis(30), Duration::from_millis(150));
        }
        // One more turn so the active-state redraw has landed.
        host.platform().pump(Duration::from_millis(50), Duration::from_millis(150));
        is_key(host, window)
    }

    /// The window as the window server composites it (glass included), without its shadow.
    fn screenshot(number: isize) -> Result<Vec<u8>, String> {
        let path = std::env::temp_dir().join(format!("nd-overrides-{}-{number}.png", std::process::id()));
        let status = std::process::Command::new("/usr/sbin/screencapture")
            .args(["-x", "-o", &format!("-l{number}")])
            .arg(&path)
            .status()
            .map_err(|e| format!("screencapture: {e}"))?;
        let png = std::fs::read(&path).map_err(|e| format!("screencapture wrote nothing ({status}): {e}"))?;
        let _ = std::fs::remove_file(&path);
        Ok(png)
    }

    fn check(host: &nd::appkit::AppHost, appearance: Appearance, scheme: &str) -> Result<(), String> {
        let handle = host.update(|cx| overrides::open(cx, appearance));
        settle(host);
        let window = host.ns_window(handle.id()).expect("window");
        // AppKit draws an inactive window's controls differently, and parallel tests take focus
        // from each other: capture only while this window is key in the active app.
        let mut capture = None;
        for _ in 0..10 {
            if !make_key(host, &window) {
                continue;
            }
            let shot = screenshot(window.windowNumber())?;
            if is_key(host, &window) {
                capture = Some(shot);
                break;
            }
        }
        let capture = capture.ok_or_else(|| format!("{scheme}: the window never stayed key for a capture"))?;
        let scale = window.backingScaleFactor().round() as u32;
        host.update(|cx| cx.close_window(handle.id()));
        settle(host);
        let path = golden(scheme, scale);
        if std::env::var_os("ND_UPDATE_GOLDENS").is_some_and(|v| v == "1") || !path.exists() {
            std::fs::write(&path, &capture).map_err(|e| e.to_string())?;
            println!("recorded {}", path.display());
            return Ok(());
        }
        let expected = std::fs::read(&path).map_err(|e| e.to_string())?;
        let (ew, eh, e) = decode(&expected);
        let (aw, ah, a) = decode(&capture);
        let actual_path = path.with_extension("actual.png");
        if (ew, eh) != (aw, ah) {
            let _ = std::fs::write(&actual_path, &capture);
            return Err(format!("{scheme}: size {aw}x{ah}, golden {ew}x{eh}"));
        }
        let diff = difference(&e, &a);
        if diff > PIXEL_TOLERANCE {
            let _ = std::fs::write(&actual_path, &capture);
            return Err(format!("{scheme}: {:.2}% of pixels differ (see {})", diff * 100., actual_path.display()));
        }
        let _ = std::fs::remove_file(&actual_path);
        println!("ok   {scheme} matches {} ({:.3}% off)", path.display(), diff * 100.);
        Ok(())
    }

    pub fn run() -> ! {
        let _lock = ndshell::lock::window_test("gate_m3::overrides_goldens");
        // Isolated from the user's theme files and data, so only the app's overrides apply.
        let data = std::env::temp_dir().join(format!("nd-overrides-goldens-{}", std::process::id()));
        unsafe {
            std::env::set_var("ND_APP_DATA_DIR", &data);
            std::env::set_var("ND_THEME_TOML", "");
            std::env::set_var("ND_THEME_JSON", "");
        }
        let host = overrides::app().host(|_cx| {});
        let mut failed = 0;
        for (appearance, scheme) in [(Appearance::Light, "light"), (Appearance::Dark, "dark")] {
            if let Err(e) = check(&host, appearance, scheme) {
                println!("FAIL {e}");
                failed += 1;
            }
        }
        let _ = std::fs::remove_dir_all(&data);
        if failed == 0 {
            println!("ND_OVERRIDES_GOLDENS_OK");
            std::process::exit(0);
        }
        std::process::exit(1);
    }
}
