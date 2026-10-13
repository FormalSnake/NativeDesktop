//! Tier 2 gate (ARCHITECTURE.md 16, M3): a matugen `theme.json` write recolours a running app
//! within 100 ms. The app runs in this process on the real AppKit backend with `ND_THEME_JSON`
//! pointing at a temporary file; each write swaps `primary`. A recolour has three legs, timed
//! apart: the file write to the watcher's callback, the callback to the first render carrying
//! the colour, and that render to the native layer showing it (the backend's display pass).
//! The legs after the watcher run on the main thread, so they are asserted in pump rounds, which
//! a loaded machine cannot stretch. The wall-clock budget for the whole path, and for the
//! watcher alone, is asserted only under nextest's `gate` profile. Prints `ND_MATUGEN_LIVE_OK`
//! and the latencies.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let name = "gate_m3::matugen_live_recolour";
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
    live::run();
}

#[cfg(target_os = "macos")]
mod live {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use nd::prelude::*;
    use nd::theme::color::{close, parse_hex};
    use nd::theme::{MatugenTheme, ThemeFileWatcher};

    /// The whole recolour, file write to native layer.
    const BUDGET: Duration = Duration::from_millis(100);
    /// The watcher alone: the first event of a burst is read at once.
    const WATCH_BUDGET: Duration = Duration::from_millis(50);
    /// Pump rounds from the first render carrying the colour to the layer showing it.
    const DISPLAY_ROUNDS: u32 = 3;

    /// When the swatch last rendered, and with which accent.
    static RENDERED: Mutex<Option<(Instant, Hsla)>> = Mutex::new(None);

    struct Swatch;

    impl Render for Swatch {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let accent = cx.theme().accent;
            *RENDERED.lock().unwrap() = Some((Instant::now(), accent));
            div().id("swatch").size_full().bg(accent)
        }
    }

    fn theme_json(primary: &str) -> String {
        format!(
            r##"{{"colors":{{"primary":{{"light":"{primary}","dark":"{primary}"}},"surface":{{"light":"#fafafa","dark":"#101010"}}}}}}"##
        )
    }

    /// The swatch layer's background, read off AppKit.
    fn shown(host: &nd::appkit::AppHost, node: nd::core::NodeId) -> Option<Hsla> {
        host.backend().layer_background(node)
    }

    fn ms(d: Duration) -> f64 {
        d.as_secs_f64() * 1e3
    }

    pub fn run() -> ! {
        let gate = std::env::var("NEXTEST_PROFILE").is_ok_and(|p| p == "gate");
        let dir = std::env::temp_dir().join(format!("nd-matugen-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("theme.json");
        std::fs::write(&path, theme_json("#336699")).expect("first theme.json");
        unsafe {
            std::env::set_var("ND_APP_DATA_DIR", &dir);
            std::env::set_var("ND_THEME_JSON", &path);
            std::env::set_var("ND_THEME_TOML", "");
        }
        // A second watcher on the same file, the app's own code, stamps when a read lands.
        let reads: Arc<Mutex<Vec<Instant>>> = Arc::default();
        let probe = {
            let reads = reads.clone();
            ThemeFileWatcher::spawn(&path, MatugenTheme::parse, move |_| reads.lock().unwrap().push(Instant::now()))
                .expect("probe watcher")
        };
        let mut handle = None;
        let host = Application::new(nd::manifest!()).host(|cx| {
            handle =
                Some(cx.open_window(
                    WindowOptions::main("swatch").title("Swatch").size(size(px(200.), px(120.))),
                    |_, cx| cx.new(|_| Swatch),
                ));
        });
        let handle = handle.expect("window");
        let platform = host.platform();
        // `pump` returns at the first idle round, so a quiet gap has to be waited out.
        let pump = |for_: Duration| {
            let end = Instant::now() + for_;
            while Instant::now() < end {
                platform.pump(Duration::from_millis(1), Duration::from_millis(2));
            }
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        let node = loop {
            pump(Duration::from_millis(20));
            if let Some(node) = host.update(|cx| cx.view_node(handle.root().entity_id())) {
                break node;
            }
            assert!(Instant::now() < deadline, "the swatch never mounted");
        };
        // The watchers' first reads and the first frame.
        pump(Duration::from_millis(300));

        let colours = ["#cc3344", "#22aa66", "#8844dd", "#dd8822", "#1188cc", "#aa2277", "#557711", "#3366ff"];
        let (mut worst, mut worst_watch) = (Duration::ZERO, Duration::ZERO);
        let mut failed = 0;
        for hex in colours {
            let want = parse_hex(hex).expect("hex");
            let seen_reads = reads.lock().unwrap().len();
            let started = Instant::now();
            std::fs::write(&path, theme_json(hex)).expect("theme.json write");
            let (mut watched, mut rendered, mut rounds_after_render) = (None, None, 0u32);
            let mut displayed = None;
            while started.elapsed() < Duration::from_secs(2) {
                platform.pump(Duration::from_millis(1), Duration::from_millis(2));
                if watched.is_none() {
                    watched = reads.lock().unwrap().get(seen_reads).copied();
                }
                if rendered.is_none() {
                    rendered = RENDERED.lock().unwrap().filter(|(_, c)| close(*c, want)).map(|(t, _)| t);
                }
                if rendered.is_some() {
                    rounds_after_render += 1;
                }
                if shown(&host, node).is_some_and(|c| close(c, want)) {
                    displayed = Some(Instant::now());
                    break;
                }
            }
            let Some(displayed) = displayed else {
                println!("FAIL {hex} never reached the window");
                failed += 1;
                pump(Duration::from_millis(150));
                continue;
            };
            let total = displayed - started;
            let watch = watched.map(|w| w.saturating_duration_since(started));
            let display = rendered.map(|r| displayed.saturating_duration_since(r));
            let mut problems = Vec::new();
            if rounds_after_render > DISPLAY_ROUNDS {
                problems.push(format!("{rounds_after_render} pump rounds from render to layer"));
            }
            if gate && total > BUDGET {
                problems.push(format!("end to end over {} ms", BUDGET.as_millis()));
            }
            if gate && watch.is_none_or(|w| w > WATCH_BUDGET) {
                problems.push(format!("watcher over {} ms", WATCH_BUDGET.as_millis()));
            }
            println!(
                "{} {hex} total {:.1} ms, watcher {} ms, render to layer {} ms ({rounds_after_render} rounds)",
                if problems.is_empty() { "ok  " } else { "SLOW" },
                ms(total),
                watch.map_or("?".into(), |w| format!("{:.1}", ms(w))),
                display.map_or("?".into(), |d| format!("{:.1}", ms(d))),
            );
            for p in &problems {
                println!("     {p}");
            }
            failed += usize::from(!problems.is_empty());
            worst = worst.max(total);
            worst_watch = worst_watch.max(watch.unwrap_or_default());
            // Past the debounce's trailing read, so the next write opens a new burst.
            pump(nd::theme::watcher::DEBOUNCE * 3);
        }
        drop(probe);
        let _ = std::fs::remove_dir_all(&dir);
        println!(
            "worst {:.1} ms end to end, {:.1} ms watcher ({})",
            ms(worst),
            ms(worst_watch),
            if gate { "wall-clock budgets enforced" } else { "wall-clock budgets reported only" }
        );
        if failed == 0 {
            println!("ND_MATUGEN_LIVE_OK");
            std::process::exit(0);
        }
        std::process::exit(1);
    }
}
