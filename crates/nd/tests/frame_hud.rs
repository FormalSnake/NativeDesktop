//! Tier 2: the frame HUD in a real window. F12's action cycles it hidden, minimal and full, and
//! it shows the window's recorded frames: rate, frame time, the slowest frame and the drops (the
//! arithmetic is `FrameSummary`'s unit test in nd-diag).
//! Prints `ND_FRAME_HUD_OK`.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let name = "nd::frame_hud";
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
    hud::run();
}

#[cfg(target_os = "macos")]
mod hud {
    use std::time::{Duration, Instant};

    use nd::appkit::AppHost;
    use nd::hud::{FrameHud, HudMode, frame_hud_toggle};
    use nd::prelude::*;

    struct Shell {
        hud: Entity<FrameHud>,
    }

    impl Render for Shell {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            scaffold()
                .on_action(frame_hud_toggle(&self.hud))
                .content(div().size_full().child(text("Content")))
                .overlay(self.hud.clone())
        }
    }

    struct Checks(u32);

    impl Checks {
        fn check(&mut self, ok: bool, what: &str) {
            println!("{} {what}", if ok { "ok  " } else { "FAIL" });
            if !ok {
                self.0 += 1;
            }
        }
    }

    fn pump(host: &AppHost, for_: Duration) {
        let end = Instant::now() + for_;
        while Instant::now() < end {
            host.platform().pump(Duration::from_millis(10), Duration::from_millis(50));
            host.update(|app| {
                if app.needs_frame() {
                    app.frame();
                }
            });
        }
    }

    pub fn run() -> ! {
        let _lock = ndshell::lock::window_test("nd::frame_hud");
        let dir = std::env::temp_dir().join(format!("nd-frame-hud-{}", std::process::id()));
        unsafe {
            std::env::set_var("ND_APP_DATA_DIR", &dir);
        }
        // `run` starts diagnostics, which keep the frames the HUD reads; a test host does not.
        nd::diag::init(nd::diag::Config::new("nd-frame-hud", "0.0.0", "appkit"));
        let mut main = None;
        let host = Application::new(nd::manifest!()).host(|cx| {
            let hud = cx.new(FrameHud::new);
            main = Some(
                cx.open_window(WindowOptions::main("hud").title("HUD").size(size(px(640.), px(420.))), move |_, cx| {
                    cx.new(|_| Shell { hud })
                }),
            );
        });
        let main = main.expect("main window");
        let id = main.id();
        pump(&host, Duration::from_millis(400));
        let texts = |host: &AppHost| -> Vec<String> {
            host.update(|app| {
                app.semantic_window(id)
                    .map(|w| w.root.iter().map(|n| n.name.to_string()).filter(|n| !n.is_empty()).collect())
                    .unwrap_or_default()
            })
        };
        let mode = |host: &AppHost| host.update(|app| main.root().read(app).hud.read(app).mode());
        let toggle = |host: &AppHost| {
            host.update(|app| app.dispatch_action(id, &nd::actions::ToggleFrameHud));
            pump(host, Duration::from_millis(150));
        };
        let mut c = Checks(0);
        c.check(mode(&host) == HudMode::Hidden, "the HUD starts hidden");
        c.check(!texts(&host).iter().any(|t| t.contains("fps")), "and shows nothing");
        let f12 = nd::actions::standard_bindings().iter().any(|b| {
            b.action().partial_eq(&nd::actions::ToggleFrameHud) && b.keystrokes().iter().map(|k| k.key()).eq(["f12"])
        });
        c.check(f12, "F12 is bound to it");

        toggle(&host);
        let shown = texts(&host);
        c.check(mode(&host) == HudMode::Minimal, "F12 shows the minimal HUD");
        let fps = shown.iter().find_map(|t| t.strip_suffix(" fps")?.parse::<f64>().ok());
        c.check(fps.is_some_and(|f| f > 0.), &format!("its rate comes from the window's recorded frames ({shown:?})"));

        toggle(&host);
        let full = texts(&host);
        c.check(mode(&host) == HudMode::Full, "F12 again shows the full HUD");
        c.check(
            full.iter().any(|t| t.ends_with(" ms budget")) && full.iter().any(|t| t.ends_with(" ms slowest")),
            &format!("it adds frame time against the budget and the slowest frame ({full:?})"),
        );
        c.check(full.iter().any(|t| t.ends_with(" dropped")), "and the dropped frames");

        toggle(&host);
        c.check(mode(&host) == HudMode::Hidden, "F12 a third time hides it");
        c.check(!texts(&host).iter().any(|t| t.contains("fps")), "and its lines go");

        if c.0 == 0 {
            println!("ND_FRAME_HUD_OK");
            std::process::exit(0);
        }
        println!("ND_FRAME_HUD_FAILED {} check(s)", c.0);
        std::process::exit(1);
    }
}
