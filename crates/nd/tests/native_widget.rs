//! Tier 2: app-defined native widgets on AppKit (ARCHITECTURE.md 3.5, 3.9). A widget with an
//! objc2 implementation is its own view in the window, laid out and updated from its props; a
//! widget whose Swift symbols are not in the binary falls back to its drawn tree. Both carry
//! their `semantics()`. Prints `ND_NATIVE_WIDGET_OK`.

#[cfg_attr(not(target_os = "macos"), allow(clippy::needless_return))]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let name = "gate_m6::appkit_native_widget";
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
    gate::run();
}

#[cfg(target_os = "macos")]
mod gate {
    use std::time::{Duration, Instant};

    use nd::appkit::AppHost;
    use nd::appkit::objc2::Message;
    use nd::appkit::objc2::rc::Retained;
    use nd::appkit::objc2_app_kit::{NSLevelIndicator, NSView};
    use nd::native::appkit::{AppKitCx, AppKitWidget, AvailableSize};
    use nd::native::*;
    use nd::prelude::*;

    struct Level;

    impl NativeWidget for Level {
        type Props = f64;
        type Event = std::convert::Infallible;

        fn semantics(value: &f64) -> Semantics {
            Semantics::new(Role::ProgressIndicator).label("Level").value(SemanticValue::range(*value, 0., 10.))
        }

        fn drawn(_: &f64, _: Emit<Self::Event>, _: &mut Window, _: &mut App) -> AnyElement {
            text("drawn level").into_any_element()
        }

        fn backends() -> Backends<Self> {
            Backends::drawn_only().appkit(LevelKit)
        }
    }

    struct LevelKit;

    impl AppKitWidget<Level> for LevelKit {
        type View = NSLevelIndicator;
        type State = ();

        fn create(&self, value: &f64, cx: &mut AppKitCx<Level>) -> (Retained<NSLevelIndicator>, ()) {
            let view = NSLevelIndicator::new(cx.mtm());
            view.setMaxValue(10.);
            view.setDoubleValue(*value);
            (view, ())
        }

        fn update(&self, view: &NSLevelIndicator, _: &mut (), _: &f64, new: &f64, _: &mut AppKitCx<Level>) {
            view.setDoubleValue(*new);
        }

        fn measure(&self, _: &NSLevelIndicator, _: &f64, _: AvailableSize) -> Size<Pixels> {
            size(px(120.), px(18.))
        }
    }

    #[derive(IntoElement)]
    struct LevelElement {
        base: NativeBase<Level>,
        value: f64,
    }

    impl NativeElement for LevelElement {
        type Widget = Level;

        fn base_mut(&mut self) -> &mut NativeBase<Level> {
            &mut self.base
        }

        fn into_parts(self) -> (NativeBase<Level>, f64) {
            (self.base, self.value)
        }
    }

    /// Registers a Swift view this binary does not link.
    struct Unlinked;

    #[derive(Clone, PartialEq, serde::Serialize)]
    struct UnlinkedProps;

    impl NativeWidget for Unlinked {
        type Props = UnlinkedProps;
        type Event = std::convert::Infallible;

        fn semantics(_: &UnlinkedProps) -> Semantics {
            Semantics::new(Role::Image).label("Unlinked")
        }

        fn drawn(_: &UnlinkedProps, _: Emit<Self::Event>, _: &mut Window, _: &mut App) -> AnyElement {
            text("drawn instead").into_any_element()
        }

        fn backends() -> Backends<Self> {
            Backends::drawn_only().swift(SwiftWidget::named("NdGateUnlinked"))
        }
    }

    #[derive(IntoElement)]
    struct UnlinkedElement {
        base: NativeBase<Unlinked>,
    }

    impl NativeElement for UnlinkedElement {
        type Widget = Unlinked;

        fn base_mut(&mut self) -> &mut NativeBase<Unlinked> {
            &mut self.base
        }

        fn into_parts(self) -> (NativeBase<Unlinked>, UnlinkedProps) {
            (self.base, UnlinkedProps)
        }
    }

    struct Shell {
        value: f64,
    }

    impl Render for Shell {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            v_flex()
                .p_4()
                .gap_2()
                .child(LevelElement { base: NativeBase::new("level"), value: self.value })
                .child(UnlinkedElement { base: NativeBase::new("unlinked") })
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

    fn level_in(view: &NSView) -> Option<Retained<NSLevelIndicator>> {
        if let Some(level) = view.downcast_ref::<NSLevelIndicator>() {
            return Some(level.retain());
        }
        view.subviews().iter().find_map(|s| level_in(&s))
    }

    pub fn run() -> ! {
        let _lock = ndshell::lock::window_test("gate_m6::appkit_native_widget");
        let mut main = None;
        let host = Application::new(nd::manifest!()).host(|cx| {
            main = Some(cx.open_window(
                WindowOptions::main("native-widget").title("Native widget").size(size(px(400.), px(240.))),
                |_, cx| cx.new(|_| Shell { value: 3. }),
            ));
        });
        let main = main.expect("main window");
        pump(&host, Duration::from_millis(500));
        let window = host.ns_window(main.id()).expect("window");
        let content = window.contentView().expect("content view");
        let mut c = Checks(0);

        let level = level_in(&content);
        c.check(level.is_some(), "the objc2 widget's NSLevelIndicator is in the window");
        if let Some(level) = &level {
            c.check(level.doubleValue() == 3., "it was created from its props");
            let frame = level.frame();
            // The column stretches its children across; the height is the measured one.
            c.check(
                (frame.size.width, frame.size.height) == (368., 18.),
                &format!("it is framed by layout from its measured size ({:?})", frame.size),
            );
        }

        host.update(|app| {
            main.root().update(app, |shell, cx| {
                shell.value = 7.;
                cx.notify();
            })
        });
        pump(&host, Duration::from_millis(300));
        c.check(level.as_ref().is_some_and(|l| l.doubleValue() == 7.), "new props reach the view through update");

        let tree = host.update(|app| app.semantic_tree());
        let node = |id: &str| tree.nodes().find(|(_, n)| n.test_id.as_deref() == Some(id)).map(|(_, n)| n.clone());
        let level_node = node("level");
        c.check(
            level_node.as_ref().is_some_and(|n| {
                (n.role, n.name.as_str(), n.value.as_deref()) == (Role::ProgressIndicator, "Level", Some("7"))
            }),
            &format!("the native leaf carries semantics() ({level_node:?})"),
        );
        let unlinked = node("unlinked");
        c.check(
            unlinked.as_ref().is_some_and(|n| n.role == Role::Image && n.subtree_text().contains("drawn instead")),
            &format!("a Swift widget whose symbols are missing is drawn, with its semantics ({unlinked:?})"),
        );

        if c.0 == 0 {
            println!("ND_NATIVE_WIDGET_OK");
            std::process::exit(0);
        }
        println!("ND_NATIVE_WIDGET_FAILED {} check(s)", c.0);
        std::process::exit(1);
    }
}
