//! Tier 2: the seam contract on a real window. Ops go through `nd_ui::transaction::Transaction`
//! exactly as the reconciler sends them, and every check reads AppKit's own state back.
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
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use nd_backend_appkit::objc2_app_kit::{NSButton, NSView};
    use nd_backend_appkit::objc2_foundation::MainThreadMarker;
    use nd_backend_appkit::props::{ButtonProps, Color, NativeButton, TextProps, ViewProps};
    use nd_backend_appkit::{AppKitBackend, HostOptions, NativeEventKind, NdAppKitPlatform, WindowHost};
    use nd_ui::backend::{AvailableSpace, Constraint, NodeKind, Props, PropsPatch, WidgetKind};
    use nd_ui::transaction::{MeasureCache, Op, Transaction};
    use nd_ui::{NodeId, Rect, WindowId};

    const W: WindowId = WindowId(7);
    const ROOT: NodeId = NodeId(1);
    const BOX: NodeId = NodeId(2);
    const LABEL: NodeId = NodeId(3);
    const BUTTON: NodeId = NodeId(4);
    const OTHER: NodeId = NodeId(5);

    struct Checks(u32);

    impl Checks {
        fn check(&mut self, ok: bool, what: &str) {
            println!("{} {what}", if ok { "ok  " } else { "FAIL" });
            if !ok {
                self.0 += 1;
            }
        }
    }

    fn props<T: std::fmt::Debug + PartialEq + 'static>(p: T) -> Props {
        Rc::new(p)
    }

    fn subviews(view: &NSView) -> Vec<*const NSView> {
        view.subviews().iter().map(|v| &*v as *const NSView).collect()
    }

    pub fn run() -> ! {
        let mtm = MainThreadMarker::new().expect("main thread");
        let platform = NdAppKitPlatform::new(mtm);
        let window = platform.open_window("Seam", 400., 300.);
        let backend = Rc::new(RefCell::new(AppKitBackend::new(mtm)));
        let _host = WindowHost::new(&backend, W, &window, HostOptions::default());
        let mut cache = MeasureCache::default();
        let mut c = Checks(0);

        let label = TextProps::new("Hello from nd");
        let mut tx = Transaction::new(W);
        tx.push(Op::Create { node: ROOT, kind: NodeKind::Root, props: props(()) });
        tx.push(Op::Create {
            node: BOX,
            kind: NodeKind::View,
            props: props(ViewProps { background: Some(Color::Accent), corner_radius: 8., ..Default::default() }),
        });
        tx.push(Op::Create { node: LABEL, kind: NodeKind::Text, props: props(label.clone()) });
        tx.push(Op::Create {
            node: BUTTON,
            kind: NodeKind::Widget(WidgetKind::of::<NativeButton>()),
            props: props(ButtonProps::new("Press")),
        });
        tx.push(Op::Insert { parent: ROOT, node: BOX, before: None });
        tx.push(Op::Insert { parent: BOX, node: LABEL, before: None });
        tx.push(Op::Insert { parent: BOX, node: BUTTON, before: Some(LABEL) });
        tx.push(Op::SetFrame { node: BOX, frame: Rect::new(10.25, 20., 200., 100.) });
        tx.push(Op::SetFrame { node: LABEL, frame: Rect::new(4., 4., 150., 20.) });
        tx.push(Op::SetFrame { node: BUTTON, frame: Rect::new(4., 40., 100., 32.) });
        tx.commit(&mut *backend.borrow_mut(), &mut cache);

        let b = backend.borrow();
        let root = window.contentView().unwrap();
        let box_view = b.view(BOX).unwrap();
        let label_view = b.view(LABEL).unwrap();
        let button_view = b.view(BUTTON).unwrap();
        c.check(subviews(&root) == vec![&*box_view as *const NSView], "root holds the box");
        c.check(
            subviews(&box_view) == vec![&*button_view as *const NSView, &*label_view as *const NSView],
            "insert before a sibling orders subviews",
        );
        c.check(button_view.downcast_ref::<NSButton>().is_some(), "the button node is AppKit's NSButton");
        let f = box_view.frame();
        c.check(
            f.origin.y == 20. && f.size.height == 100. && f.origin.x <= 10.25 && f.origin.x * 2. == (f.origin.x * 2.).round(),
            "frames are top-down and rounded outward onto the backing grid",
        );
        drop(b);

        let measured = cache.measure(
            &mut *backend.borrow_mut(),
            LABEL,
            Constraint { width: AvailableSpace::MaxContent, height: AvailableSpace::MaxContent },
        );
        c.check(measured.width > 40. && measured.height >= 12., &format!("text measures with CoreText ({measured:?})"));
        let button_size = cache.measure(
            &mut *backend.borrow_mut(),
            BUTTON,
            Constraint { width: AvailableSpace::MaxContent, height: AvailableSpace::MaxContent },
        );
        c.check(button_size.height > 15., &format!("the button reports its own fitting size ({button_size:?})"));

        let mut tx = Transaction::new(W);
        tx.push(Op::Update {
            node: LABEL,
            patch: PropsPatch { old: props(label.clone()), new: props(TextProps::new("A much longer label for nd")) },
        });
        tx.commit(&mut *backend.borrow_mut(), &mut cache);
        let wider = cache.measure(
            &mut *backend.borrow_mut(),
            LABEL,
            Constraint { width: AvailableSpace::MaxContent, height: AvailableSpace::MaxContent },
        );
        c.check(wider.width > measured.width, "an update drops the cached measurement");

        backend.borrow().events().set_waker(|| {});
        unsafe { button_view.downcast_ref::<NSButton>().unwrap().performClick(None) };
        platform.pump(Duration::from_millis(50), Duration::from_secs(1));
        let events = backend.borrow().events().drain();
        c.check(
            events.len() == 1 && events[0].node == BUTTON && events[0].kind == NativeEventKind::Click,
            "a click reaches the event queue with the node id",
        );

        let mut tx = Transaction::new(W);
        tx.push(Op::Create { node: OTHER, kind: NodeKind::View, props: props(ViewProps::default()) });
        tx.push(Op::Insert { parent: ROOT, node: OTHER, before: None });
        tx.push(Op::Move { node: LABEL, parent: OTHER, before: None });
        tx.commit(&mut *backend.borrow_mut(), &mut cache);
        let other_view = backend.borrow().view(OTHER).unwrap();
        c.check(
            backend.borrow().view(LABEL).is_some_and(|v| std::ptr::eq(&*v, &*label_view))
                && subviews(&other_view) == vec![&*label_view as *const NSView],
            "move keeps the same NSView under its new parent",
        );

        let mut tx = Transaction::new(W);
        tx.push(Op::Remove { node: BOX });
        tx.push(Op::Release { node: BOX });
        tx.push(Op::Release { node: BUTTON });
        tx.commit(&mut *backend.borrow_mut(), &mut cache);
        c.check(
            unsafe { box_view.superview() }.is_none() && backend.borrow().view(BOX).is_none(),
            "remove detaches, release drops the node",
        );

        window.close();
        if c.0 == 0 {
            println!("ND_BACKEND_SEAM_OK");
            std::process::exit(0);
        }
        println!("ND_BACKEND_SEAM_FAILED {} check(s)", c.0);
        std::process::exit(1);
    }
}
