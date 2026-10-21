//! Tier 2 goldens of the media page on macOS, light and dark: the gallery window in this process
//! on the real AppKit backend, captured as the window server composites it (`screencapture -l`)
//! and compared with `tests/goldens/media-macos-<scheme>-<page>@<scale>x.png`. Needs Screen Recording
//! for the process that runs the test, and holds the ndshell machine lock while the window is
//! key.
//!
//! `ND_UPDATE_GOLDENS=1` rewrites the goldens. A mismatch writes `<golden>.actual.png` beside
//! the golden. Prints `ND_GALLERY_GOLDENS_OK`.

#[cfg_attr(not(target_os = "macos"), allow(clippy::needless_return))]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let name = "gate_m6::gallery_media_goldens";
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

    use nd::appkit::objc2::Message;
    use nd::appkit::objc2::rc::Retained;
    use nd::appkit::objc2_app_kit::{NSProgressIndicator, NSScrollView, NSView};
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

    /// At most this many screenfuls; capturing stops once the page scrolls no further.
    const PAGES: usize = 6;

    fn golden(scheme: &str, page: usize, scale: u32) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/goldens/media-macos-{scheme}-{page}@{scale}x.png"))
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
        // A few more turns so deferred work (toolbar items, traffic lights) and decoded images land.
        host.platform().pump(Duration::from_millis(300), Duration::from_millis(800));
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
        // Long enough for the active-state redraw to land and for the overlay scrollers AppKit
        // flashes when a window with overflowing content becomes key to fade out again.
        host.platform().pump(Duration::from_millis(1200), Duration::from_millis(1500));
        is_key(host, window)
    }

    /// The window as the window server composites it (glass included), without its shadow.
    fn screenshot(number: isize) -> Result<Vec<u8>, String> {
        let path = std::env::temp_dir().join(format!("nd-gallery-{}-{number}.png", std::process::id()));
        let status = std::process::Command::new("/usr/sbin/screencapture")
            .args(["-x", "-o", &format!("-l{number}")])
            .arg(&path)
            .status()
            .map_err(|e| format!("screencapture: {e}"))?;
        let png = std::fs::read(&path).map_err(|e| format!("screencapture wrote nothing ({status}): {e}"))?;
        let _ = std::fs::remove_file(&path);
        Ok(png)
    }

    /// The form's scroll view: the scroll view under the window with the tallest document.
    fn form_scroller(view: &NSView) -> Option<Retained<NSScrollView>> {
        let mut best: Option<Retained<NSScrollView>> = None;
        let mut stack = vec![view.retain()];
        while let Some(v) = stack.pop() {
            if let Some(scroll) = v.downcast_ref::<NSScrollView>()
                && let Some(doc) = scroll.documentView()
                && best
                    .as_ref()
                    .is_none_or(|b| b.documentView().is_some_and(|d| d.frame().size.height < doc.frame().size.height))
            {
                best = Some(scroll.retain());
            }
            stack.extend(v.subviews().iter());
        }
        best
    }

    /// Stops indeterminate progress animations, so the captures do not depend on when they run.
    fn still(view: &NSView) {
        let mut stack = vec![view.retain()];
        while let Some(v) = stack.pop() {
            if let Some(p) = v.downcast_ref::<NSProgressIndicator>()
                && p.isIndeterminate()
            {
                // SAFETY: main thread; the sender argument is unused.
                unsafe { p.stopAnimation(None) };
            }
            stack.extend(v.subviews().iter());
        }
    }

    /// A capture while the window is key in the active app; AppKit draws inactive controls
    /// differently, and parallel tests take focus from each other.
    fn capture(host: &nd::appkit::AppHost, window: &nd::appkit::NdWindow, scheme: &str) -> Result<Vec<u8>, String> {
        for _ in 0..10 {
            if !make_key(host, window) {
                continue;
            }
            let shot = screenshot(window.windowNumber())?;
            if is_key(host, window) {
                return Ok(shot);
            }
        }
        Err(format!("{scheme}: the window never stayed key for a capture"))
    }

    fn compare(path: &Path, capture: &[u8], what: &str) -> Result<(), String> {
        if std::env::var_os("ND_UPDATE_GOLDENS").is_some_and(|v| v == "1") || !path.exists() {
            std::fs::write(path, capture).map_err(|e| e.to_string())?;
            println!("recorded {}", path.display());
            return Ok(());
        }
        let expected = std::fs::read(path).map_err(|e| e.to_string())?;
        let (ew, eh, e) = decode(&expected);
        let (aw, ah, a) = decode(capture);
        let actual_path = path.with_extension("actual.png");
        if (ew, eh) != (aw, ah) {
            let _ = std::fs::write(&actual_path, capture);
            return Err(format!("{what}: size {aw}x{ah}, golden {ew}x{eh}"));
        }
        let diff = difference(&e, &a);
        if diff > PIXEL_TOLERANCE {
            let _ = std::fs::write(&actual_path, capture);
            return Err(format!("{what}: {:.2}% of pixels differ (see {})", diff * 100., actual_path.display()));
        }
        let _ = std::fs::remove_file(&actual_path);
        println!("ok   {what} matches {} ({:.3}% off)", path.display(), diff * 100.);
        Ok(())
    }

    /// The page from the top, then one capture per screenful as the form scrolls, so every
    /// section is in a golden.
    fn check(host: &nd::appkit::AppHost, appearance: Appearance, scheme: &str) -> Result<(), String> {
        let start = gallery::Start { page: gallery::Page::Media, appearance: Some(appearance), ..Default::default() };
        let handle = host.update(|cx| gallery::open(cx, start));
        settle(host);
        let window = host.ns_window(handle.id()).expect("window");
        let scale = window.backingScaleFactor().round() as u32;
        let content = window.contentView().expect("content view");
        let scroller = form_scroller(&content).ok_or_else(|| format!("{scheme}: the page has no scroll view"))?;
        let mut failures = Vec::new();
        for page in 0..PAGES {
            if page > 0 {
                let clip = scroller.contentView();
                // The form's own frame can run past the window; step by what is on screen.
                let step = clip.visibleRect().size.height * 0.8;
                let before = clip.bounds().origin;
                let mut origin = before;
                origin.y += step;
                clip.scrollToPoint(origin);
                scroller.reflectScrolledClipView(&clip);
                settle(host);
                let fixed = clip.constrainBoundsRect(clip.bounds()).origin;
                if (fixed.y - clip.bounds().origin.y).abs() >= 1. {
                    clip.scrollToPoint(fixed);
                    scroller.reflectScrolledClipView(&clip);
                    settle(host);
                }
                // The end is where scrolling stops moving the clip.
                if (clip.bounds().origin.y - before.y).abs() < 1. {
                    break;
                }
            }
            still(&content);
            let shot = capture(host, &window, scheme)?;
            if let Err(e) = compare(&golden(scheme, page, scale), &shot, &format!("{scheme} {page}")) {
                failures.push(e);
            }
        }
        host.update(|cx| cx.close_window(handle.id()));
        settle(host);
        if failures.is_empty() { Ok(()) } else { Err(failures.join("; ")) }
    }

    pub fn run() -> ! {
        // Isolated from the user's theme files and data, so only the app's overrides apply.
        let data = std::env::temp_dir().join(format!("nd-gallery-media-goldens-{}", std::process::id()));
        unsafe {
            std::env::set_var("ND_APP_DATA_DIR", &data);
            std::env::set_var("ND_THEME_TOML", "");
            std::env::set_var("ND_THEME_JSON", "");
        }
        let _lock = ndshell::lock::window_test("gate_m6::gallery_media_goldens");
        let host = gallery::app().host(|_cx| {});
        let mut failed = 0;
        for (appearance, scheme) in [(Appearance::Light, "light"), (Appearance::Dark, "dark")] {
            if let Err(e) = check(&host, appearance, scheme) {
                println!("FAIL {e}");
                failed += 1;
            }
        }
        let _ = std::fs::remove_dir_all(&data);
        if failed == 0 {
            println!("ND_GALLERY_GOLDENS_OK");
            std::process::exit(0);
        }
        std::process::exit(1);
    }
}
