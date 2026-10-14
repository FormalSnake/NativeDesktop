//! Tier 2: GPUI text next to AppKit's. Every text style is drawn as a GPUI label under the
//! macOS theme with the fonts and text styles the OS reports, and as an `NSTextField` label in
//! `NSFont.preferredFont(forTextStyle:)`; advance width, line height and the ink of the two
//! renderings at 2x must agree. macOS only: it is the one platform with both renderers.

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::Arc;

    use nd_backend_gpui::GpuiHost;
    use nd_components::forms::*;
    use nd_gpui::gpui::{self, AppContext as _, HeadlessAppContext};
    use nd_theme::{OsTheme, Preset, Scheme, ThemeRegistry};
    use nd_ui::prelude::*;
    use nd_ui::{SemanticNode, WindowId};
    use objc2::AnyThread;
    use objc2_app_kit::{
        NSAttributedStringNSStringDrawing, NSBitmapImageRep, NSDeviceRGBColorSpace, NSFont, NSFontTextStyle,
        NSFontTextStyleBody, NSFontTextStyleCallout, NSFontTextStyleCaption1, NSFontTextStyleCaption2,
        NSFontTextStyleFootnote, NSFontTextStyleHeadline, NSFontTextStyleLargeTitle, NSFontTextStyleSubheadline,
        NSFontTextStyleTitle1, NSFontTextStyleTitle2, NSFontTextStyleTitle3, NSLayoutManager, NSTextField,
    };
    use objc2_foundation::{MainThreadMarker, NSDictionary, NSPoint, NSRect, NSString};

    const SAMPLE: &str = "Hamburgefonstiv 0123 Wafer";
    const SCALE: f64 = 2.;

    /// Advance widths and line heights, in points.
    const METRIC_TOLERANCE: f32 = 0.05;
    /// Ink box size and its top and bottom from the baseline, in device pixels: a glyph edge
    /// lands on either side of a pixel boundary.
    const INK_BOX_TOLERANCE: i32 = 1;
    /// Mean difference of the normalised ink over the ink box. CoreGraphics draws AppKit's glyphs
    /// at their exact subpixel position; GPUI rasterises each glyph at one of four horizontal
    /// offsets and on whole device pixel rows, which moves edges by up to half a pixel.
    const INK_TOLERANCE: f32 = 0.15;

    fn style_name(style: TextStyle) -> &'static str {
        match style {
            TextStyle::LargeTitle => "largeTitle",
            TextStyle::Title1 => "title1",
            TextStyle::Title2 => "title2",
            TextStyle::Title3 => "title3",
            TextStyle::Headline => "headline",
            TextStyle::Subheadline => "subheadline",
            TextStyle::Body => "body",
            TextStyle::Callout => "callout",
            TextStyle::Footnote => "footnote",
            TextStyle::Caption1 => "caption1",
            TextStyle::Caption2 => "caption2",
        }
    }

    fn ns_style(style: TextStyle) -> &'static NSFontTextStyle {
        unsafe {
            match style {
                TextStyle::LargeTitle => NSFontTextStyleLargeTitle,
                TextStyle::Title1 => NSFontTextStyleTitle1,
                TextStyle::Title2 => NSFontTextStyleTitle2,
                TextStyle::Title3 => NSFontTextStyleTitle3,
                TextStyle::Headline => NSFontTextStyleHeadline,
                TextStyle::Subheadline => NSFontTextStyleSubheadline,
                TextStyle::Body => NSFontTextStyleBody,
                TextStyle::Callout => NSFontTextStyleCallout,
                TextStyle::Footnote => NSFontTextStyleFootnote,
                TextStyle::Caption1 => NSFontTextStyleCaption1,
                TextStyle::Caption2 => NSFontTextStyleCaption2,
            }
        }
    }

    struct Styles;

    impl Render for Styles {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            v_flex()
                .size_full()
                .items_start()
                .p(px(16.))
                .gap(px(12.))
                .children(TextStyle::ALL.map(|s| label(SAMPLE).text_style(s).id(style_name(s))))
        }
    }

    /// Normalised ink (0 to 1) of a `w` x `h` device pixel rendering, row-major.
    struct Ink {
        w: usize,
        h: usize,
        ink: Vec<f32>,
    }

    impl Ink {
        fn normalised(w: usize, h: usize, raw: Vec<f32>) -> Ink {
            let max = raw.iter().copied().fold(0., f32::max).max(f32::EPSILON);
            Ink { w, h, ink: raw.into_iter().map(|v| v / max).collect() }
        }

        fn at(&self, x: usize, y: usize) -> f32 {
            if x < self.w && y < self.h { self.ink[y * self.w + x] } else { 0. }
        }

        /// Left, top, right, bottom of the pixels past a quarter of full ink.
        fn bounds(&self) -> (i32, i32, i32, i32) {
            let (mut l, mut t, mut r, mut b) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
            for y in 0..self.h {
                for x in 0..self.w {
                    if self.at(x, y) > 0.25 {
                        (l, t, r, b) = (l.min(x as i32), t.min(y as i32), r.max(x as i32), b.max(y as i32));
                    }
                }
            }
            (l, t, r + 1, b + 1)
        }
    }

    /// Mean difference of two inks, each taken from its own ink box's corner.
    fn ink_difference(a: &Ink, b: &Ink) -> f32 {
        let (al, at, ar, ab) = a.bounds();
        let (bl, bt, br, bb) = b.bounds();
        let (w, h) = ((ar - al).max(br - bl) as usize, (ab - at).max(bb - bt) as usize);
        let mut sum = 0.;
        for y in 0..h {
            for x in 0..w {
                let pa = a.at(x + al as usize, y + at as usize);
                let pb = b.at(x + bl as usize, y + bt as usize);
                sum += (pa - pb).abs();
            }
        }
        sum / (w * h).max(1) as f32
    }

    struct Measured {
        /// Text advance, points.
        advance: f32,
        /// Line height, points.
        line_height: f32,
        /// Baseline below the top of the text's frame, points.
        baseline: f32,
        ink: Ink,
    }

    fn appkit(style: TextStyle, mtm: MainThreadMarker) -> Measured {
        let field = NSTextField::labelWithString(&NSString::from_str(SAMPLE), mtm);
        let font = unsafe { NSFont::preferredFontForTextStyle_options(ns_style(style), &NSDictionary::new()) };
        field.setFont(Some(&font));
        let fitting = field.fittingSize();
        field.setFrame(NSRect::new(NSPoint::new(0., 0.), fitting));
        let (w, h) = ((fitting.width * SCALE).ceil() as usize, (fitting.height * SCALE).ceil() as usize);
        let rep = unsafe {
            NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(),
                std::ptr::null_mut(),
                w as isize,
                h as isize,
                8,
                4,
                true,
                false,
                NSDeviceRGBColorSpace,
                0,
                32,
            )
        }
        .expect("bitmap");
        rep.setSize(fitting);
        field.cacheDisplayInRect_toBitmapImageRep(field.bounds(), &rep);
        let row = rep.bytesPerRow() as usize;
        let data = unsafe { std::slice::from_raw_parts(rep.bitmapData(), row * h) };
        let raw = (0..h).flat_map(|y| (0..w).map(move |x| (y, x))).map(|(y, x)| f32::from(data[y * row + x * 4 + 3]));
        Measured {
            advance: field.attributedStringValue().size().width as f32,
            line_height: NSLayoutManager::new().defaultLineHeightForFont(&font) as f32,
            baseline: field.firstBaselineOffsetFromTop() as f32,
            ink: Ink::normalised(w, h, raw.collect()),
        }
    }

    /// Ink width, ink height, and the ink's top and bottom from the baseline, in device pixels.
    fn ink_metrics(m: &Measured) -> [i32; 4] {
        let (l, t, r, b) = m.ink.bounds();
        let baseline = (f64::from(m.baseline) * SCALE).round() as i32;
        [r - l, b - t, t - baseline, b - baseline]
    }

    /// Both inks over each other, aligned at their ink boxes and scaled 4x: AppKit's in red,
    /// GPUI's in blue, both in black.
    fn write_overlay(name: &str, drawn: &Ink, native: &Ink) -> std::path::PathBuf {
        let ((dl, dt, dr, db), (nl, nt, nr, nb)) = (drawn.bounds(), native.bounds());
        let (w, h) = ((dr - dl).max(nr - nl) as u32 + 4, (db - dt).max(nb - nt) as u32 + 4);
        let at = |ink: &Ink, l: i32, t: i32, x: u32, y: u32| {
            let (x, y) = (x as i32 / 4 + l - 2, y as i32 / 4 + t - 2);
            if x < 0 || y < 0 { 0. } else { ink.at(x as usize, y as usize) }
        };
        let overlay = image::RgbaImage::from_fn(w * 4, h * 4, |x, y| {
            let (g, a) = (at(drawn, dl, dt, x, y), at(native, nl, nt, x, y));
            image::Rgba([255 - (g * 255.) as u8, 255 - (a.max(g) * 255.) as u8, 255 - (a * 255.) as u8, 255])
        });
        let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("system-font-{name}.png"));
        overlay.save(&path).expect("write overlay");
        path
    }

    fn find<'a>(node: &'a SemanticNode, id: &str) -> Option<&'a SemanticNode> {
        if node.test_id.as_deref() == Some(id) {
            return Some(node);
        }
        node.children.iter().find_map(|c| find(c, id))
    }

    fn settle(cx: &mut HeadlessAppContext, host: &GpuiHost) {
        for _ in 0..4 {
            cx.run_until_parked();
            cx.update(|cx| {
                host.pump(cx);
                host.draw(cx);
            });
        }
        cx.run_until_parked();
    }

    pub fn gpui_text_matches_appkit_labels() {
        let mtm = MainThreadMarker::new().expect("tests run on the main thread");
        let text = nd_gpui::gpui_platform::current_platform(true).text_system();
        let mut cx =
            HeadlessAppContext::with_platform(text, Arc::new(()), nd_gpui::gpui_platform::current_headless_renderer);
        let mut window: Option<WindowId> = None;
        let host = cx.update(|cx| {
            GpuiHost::builder().quit_when_empty(false).start(cx, |app| {
                let handle = app.open_window(
                    WindowOptions::main("styles").title("Styles").size(size(px(640.), px(560.))),
                    |_, cx| cx.new(|_| Styles),
                );
                window = Some(handle.id());
            })
        });
        let window = window.expect("window");
        let mut registry = ThemeRegistry::new(Preset::MacOs);
        let fonts = nd_theme::fonts::resolve();
        assert_eq!(fonts.body.as_deref(), Some(".AppleSystemUIFont"), "NSFont.systemFont's family");
        assert_eq!(fonts.mono.as_deref(), Some(".AppleSystemUIFontMonospaced"));
        registry.set_os(OsTheme { scheme: Some(Scheme::Light), fonts, ..OsTheme::default() });
        let theme = registry.app_theme().clone();
        host.backend().set_theme(registry);
        settle(&mut cx, &host);
        let capture = host.job(|app| app.capture(window)).expect("capture");
        let image = image::load_from_memory(&capture.png).expect("png").to_rgba8();
        let tree = host.read(|app| app.semantic_window(window).expect("window"));
        let gpui_window = cx.update(|cx| cx.windows()[0]);

        let mut failures = Vec::new();
        println!("style        advance gpui/appkit  line gpui/appkit  ink w,h,top,bottom delta (dev px)  ink diff");
        for style in TextStyle::ALL {
            let name = style_name(style);
            let native = appkit(style, mtm);
            let bounds = find(&tree.root, name).and_then(|n| n.bounds).unwrap_or_else(|| panic!("{name} laid out"));
            let t = theme.text_styles[style];
            let font = gpui::Font { weight: t.weight, ..theme.fonts.body.clone() };
            let run = gpui::TextRun {
                len: SAMPLE.len(),
                font,
                color: gpui::black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let layout = cx
                .update(|cx| {
                    cx.update_window(gpui_window, |_, w, _| w.text_system().layout_line(SAMPLE, t.size, &[run], None))
                })
                .expect("window");
            let half_leading = (t.line_height - (layout.ascent + layout.descent)) / 2.;

            let (x0, y0) = ((f64::from(bounds.origin.x) * SCALE) as u32, (f64::from(bounds.origin.y) * SCALE) as u32);
            let (w, h) = (
                (f64::from(bounds.size.width) * SCALE).ceil() as u32,
                (f64::from(bounds.size.height) * SCALE).ceil() as u32,
            );
            let luma = |x: u32, y: u32| {
                let p = image.get_pixel(x0 + x, y0 + y);
                0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2])
            };
            let pixels = || (0..h).flat_map(|y| (0..w).map(move |x| (x, y)));
            let background = pixels().map(|(x, y)| luma(x, y)).fold(0., f32::max);
            let raw = pixels().map(|(x, y)| (background - luma(x, y)).max(0.)).collect();
            let drawn = Measured {
                advance: layout.width.as_f32(),
                line_height: bounds.size.height,
                baseline: (half_leading + layout.ascent).as_f32(),
                ink: Ink::normalised(w as usize, h as usize, raw),
            };

            let (g, a) = (ink_metrics(&drawn), ink_metrics(&native));
            let delta = [g[0] - a[0], g[1] - a[1], g[2] - a[2], g[3] - a[3]];
            let difference = ink_difference(&drawn.ink, &native.ink);
            println!(
                "{name:<12} {:>7.2} / {:<7.2}     {:>4.1} / {:<4.1}      {:>3} {:>3} {:>3} {:>3}                     {:.3}",
                drawn.advance,
                native.advance,
                drawn.line_height,
                native.line_height,
                delta[0],
                delta[1],
                delta[2],
                delta[3],
                difference
            );
            let mut fail = |why: String| failures.push(format!("{name}: {why}"));
            if (drawn.advance - native.advance).abs() > METRIC_TOLERANCE {
                fail(format!("advance {} != AppKit's {}", drawn.advance, native.advance));
            }
            if (drawn.line_height - native.line_height).abs() > METRIC_TOLERANCE {
                fail(format!("line height {} != AppKit's {}", drawn.line_height, native.line_height));
            }
            if delta.iter().any(|d| d.abs() > INK_BOX_TOLERANCE) {
                fail(format!("ink box off by {delta:?}"));
            }
            if difference > INK_TOLERANCE {
                fail(format!("ink differs by {difference:.3}"));
            }
            if std::env::var_os("ND_FONT_OVERLAYS").is_some() || failures.last().is_some_and(|f| f.starts_with(name)) {
                println!("  overlay: {}", write_overlay(name, &drawn.ink, &native.ink).display());
            }
        }
        assert!(failures.is_empty(), "GPUI text differs from AppKit's:\n{}", failures.join("\n"));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(target_os = "macos")]
    let tests: Vec<(&str, fn())> = vec![("gpui_text_matches_appkit_labels", mac::gpui_text_matches_appkit_labels)];
    #[cfg(not(target_os = "macos"))]
    let tests: Vec<(&str, fn())> = Vec::new();
    harness(&args, &tests);
}

/// The libtest command line, enough for `cargo test` and nextest (as `headless.rs`).
fn harness(args: &[String], tests: &[(&str, fn())]) {
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--list") {
        if !has("--ignored") {
            for (name, _) in tests {
                println!("{name}: test");
            }
        }
        return;
    }
    if has("--ignored") {
        return;
    }
    let exact = has("--exact");
    let mut skip = false;
    let filters: Vec<&String> = args
        .iter()
        .filter(|a| {
            if std::mem::take(&mut skip) {
                return false;
            }
            if matches!(a.as_str(), "--format" | "--test-threads" | "--color" | "--skip" | "-Z") {
                skip = true;
            }
            !a.starts_with('-')
        })
        .collect();
    let mut failed = 0;
    for (name, test) in tests {
        let selected =
            filters.is_empty() || filters.iter().any(|f| if exact { *f == name } else { name.contains(f.as_str()) });
        if !selected {
            continue;
        }
        match std::panic::catch_unwind(*test) {
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        std::process::exit(101);
    }
}
