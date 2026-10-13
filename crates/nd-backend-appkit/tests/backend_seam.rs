//! Tier 2: an `nd-ui` app on the AppKit backend and a real window. The reconciler drives the
//! seam exactly as `nd::Application::run` does; every check reads AppKit's own state back.
//! Prints `ND_BACKEND_SEAM_OK`.

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main("gate_m0::appkit_seam", || seam::run());
    #[cfg(not(target_os = "macos"))]
    harness::main("gate_m0::appkit_seam", || {});
}

#[cfg(target_os = "macos")]
mod seam {
    use std::time::Duration;

    use nd_backend_appkit::objc2::Message;
    use nd_backend_appkit::objc2_app_kit::{NSAccessibility, NSButton, NSView};
    use nd_backend_appkit::objc2_foundation::{MainThreadMarker, NSString};
    use nd_backend_appkit::{AppKitBackend, NdAppKitPlatform, WindowHost};
    use nd_ui::prelude::*;
    use nd_ui::{App, Size, WindowId};

    struct Counter {
        count: u32,
        wide: bool,
    }

    impl Render for Counter {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            v_flex()
                .size_full()
                .p(px(10.25))
                .gap_2()
                .child(text(if self.wide { "A much longer label for nd" } else { "Hello from nd" }).w(px(100.)))
                .child(text(self.count.to_string()).tabular_nums())
                .child(button("add", "Add one").on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.count += 1;
                    cx.notify();
                })))
                .child(div().id("box").w(px(120.)).h(px(40.)).rounded(px(8.)).bg(rgb(0x3366ff)))
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

    fn subviews(view: &NSView) -> Vec<String> {
        view.subviews().iter().map(|v| v.class().name().to_str().unwrap_or("?").to_owned()).collect()
    }

    fn content_root(window: &NSView) -> nd_backend_appkit::objc2::rc::Retained<NSView> {
        window.subviews().iter().next().expect("root content view")
    }

    pub fn run() -> ! {
        let mtm = MainThreadMarker::new().expect("main thread");
        let platform = NdAppKitPlatform::new(mtm);
        let backend = AppKitBackend::new(mtm);
        let mut app = App::with_backend(Box::new(backend.clone()));
        let handle =
            app.open_window(WindowOptions::main("seam"), |_, cx| cx.new(|_| Counter { count: 0, wide: false }));
        let id: WindowId = handle.id();
        let window = platform.open_window("Seam", 400., 300.);
        let _host = WindowHost::new(&backend, id, &window);
        let mut c = Checks(0);

        app.resize_window(id, Size { width: 400., height: 300. });
        app.frame();
        let root = content_root(&window.contentView().unwrap());
        let column = root.subviews().iter().next().expect("the view's column");
        let kinds = subviews(&column);
        let expected = ["NdTextView", "NdTextView", "NSButton", "NdLayerView"];
        c.check(
            kinds.len() == expected.len() && kinds.iter().zip(expected).all(|(k, e)| k.contains(e)),
            &format!("the tree mounts as AppKit views in order ({kinds:?})"),
        );
        let label = column.subviews().iter().next().unwrap();
        let f = label.frame();
        c.check(
            f.origin.x <= 10.25 && (f.origin.x * 2.).fract() == 0. && f.size.height > 12.,
            &format!("frames are top-down, measured with CoreText, rounded outward ({f:?})"),
        );
        let narrow = f.size.height;

        let count_label = column.subviews().iter().nth(1).unwrap();
        let button = column.subviews().iter().nth(2).unwrap();
        let button = button.downcast_ref::<NSButton>().expect("NSButton").retain();
        backend.on_wake(|| {});
        unsafe { button.performClick(None) };
        platform.pump(Duration::from_millis(50), Duration::from_secs(1));
        app.drain_events();
        app.frame();
        let value = count_label.accessibilityValue().and_then(|v| v.downcast::<NSString>().ok()).map(|s| s.to_string());
        c.check(
            value.as_deref() == Some("1"),
            &format!("a native click runs the handler and the label re-renders ({value:?})"),
        );

        handle.root().update(&mut app, |this, cx| {
            this.wide = true;
            cx.notify();
        });
        app.frame();
        let taller = column.subviews().iter().next().unwrap().frame().size.height;
        c.check(taller > narrow, &format!("an update re-measures the text: it wraps ({narrow} -> {taller} pt)"));
        c.check(
            column.subviews().iter().next().is_some_and(|v| std::ptr::eq(&*v, &*label)),
            "the label keeps its NSView across updates",
        );

        app.close_window(id);
        c.check(root.subviews().is_empty(), "closing the window removes every view");

        window.close();
        if c.0 == 0 {
            println!("ND_BACKEND_SEAM_OK");
            std::process::exit(0);
        }
        println!("ND_BACKEND_SEAM_FAILED {} check(s)", c.0);
        std::process::exit(1);
    }
}
