//! Tier 2 gate (ARCHITECTURE.md 16, M3): a matugen `theme.json` write recolours a running app
//! within 100 ms. The app runs in this process on the real AppKit backend with `ND_THEME_JSON`
//! pointing at a temporary file; each write swaps `primary`, and the clock stops when the
//! swatch's native layer shows the new accent, after the watcher, the main-thread hop, the
//! re-render and the backend's display pass. Prints `ND_MATUGEN_LIVE_OK` and the latencies.

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
    use std::time::{Duration, Instant};

    use nd::prelude::*;
    use nd::theme::color::{close, parse_hex};

    const BUDGET: Duration = Duration::from_millis(100);

    struct Swatch;

    impl Render for Swatch {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().id("swatch").size_full().bg(cx.theme().accent)
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

    pub fn run() -> ! {
        let dir = std::env::temp_dir().join(format!("nd-matugen-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("theme.json");
        std::fs::write(&path, theme_json("#336699")).expect("first theme.json");
        unsafe {
            std::env::set_var("ND_APP_DATA_DIR", &dir);
            std::env::set_var("ND_THEME_JSON", &path);
            std::env::set_var("ND_THEME_TOML", "");
        }
        let mut handle = None;
        let host = Application::new(nd::manifest!()).host(|cx| {
            handle =
                Some(cx.open_window(
                    WindowOptions::main("swatch").title("Swatch").size(size(px(200.), px(120.))),
                    |_, cx| cx.new(|_| Swatch),
                ));
        });
        let handle = handle.expect("window");
        let pump = |for_: Duration| host.platform().pump(Duration::from_millis(1), for_);
        let deadline = Instant::now() + Duration::from_secs(3);
        let node = loop {
            pump(Duration::from_millis(20));
            if let Some(node) = host.update(|cx| cx.view_node(handle.root().entity_id())) {
                break node;
            }
            assert!(Instant::now() < deadline, "the swatch never mounted");
        };
        // The watcher's first read and the first frame.
        pump(Duration::from_millis(300));

        let colours = ["#cc3344", "#22aa66", "#8844dd", "#dd8822", "#1188cc", "#aa2277", "#557711", "#3366ff"];
        let mut worst = Duration::ZERO;
        let mut failed = 0;
        for hex in colours {
            let want = parse_hex(hex).expect("hex");
            let started = Instant::now();
            std::fs::write(&path, theme_json(hex)).expect("theme.json write");
            let took = loop {
                pump(Duration::from_millis(2));
                if shown(&host, node).is_some_and(|c| close(c, want)) {
                    break Some(started.elapsed());
                }
                if started.elapsed() > Duration::from_secs(2) {
                    break None;
                }
            };
            match took {
                Some(t) => {
                    println!(
                        "{} {hex} recoloured in {:.1} ms",
                        if t <= BUDGET { "ok  " } else { "SLOW" },
                        t.as_secs_f64() * 1e3
                    );
                    worst = worst.max(t);
                    if t > BUDGET {
                        failed += 1;
                    }
                }
                None => {
                    println!("FAIL {hex} never reached the window");
                    failed += 1;
                }
            }
            // Give the debounce a quiet gap, as separate desktop theme changes have.
            pump(Duration::from_millis(120));
        }
        let _ = std::fs::remove_dir_all(&dir);
        println!("worst {:.1} ms (budget {} ms)", worst.as_secs_f64() * 1e3, BUDGET.as_millis());
        if failed == 0 {
            println!("ND_MATUGEN_LIVE_OK");
            std::process::exit(0);
        }
        std::process::exit(1);
    }
}
