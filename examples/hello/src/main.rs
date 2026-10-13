//! The counter from `docs/plan/examples/hello`, on the AppKit backend. The `nd-ui` reconciler
//! and Taffy have not landed, so this drives the backend seam directly: it builds the ops a
//! render of `Counter` would produce and lays the column out by hand. Once `nd-ui` core lands
//! this file becomes the design-target `main.rs`.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("hello runs on the AppKit backend only until the GPUI backend lands (M2)");
}

#[cfg(target_os = "macos")]
fn main() {
    app::run();
}

#[cfg(target_os = "macos")]
mod app {
    use std::cell::RefCell;
    use std::rc::Rc;

    use nd::appkit::objc2_foundation::MainThreadMarker;
    use nd::appkit::props::{ButtonProps, Color, NativeButton, TextProps, ViewProps};
    use nd::appkit::{AppKitBackend, HostOptions, NativeEventKind, NdAppKitPlatform, WindowHost};
    use nd::backend::{AvailableSpace, Backend, Constraint, NodeKind, Props, PropsPatch, WidgetKind};
    use nd::core::{NodeId, Point, Rect, Size, WindowId};
    use nd::transaction::{MeasureCache, Op, Transaction};

    const WINDOW: WindowId = WindowId(1);
    const ROOT: NodeId = NodeId(1);
    const SURFACE: NodeId = NodeId(2);
    const COUNT: NodeId = NodeId(3);
    const CAPTION: NodeId = NodeId(4);
    const ADD: NodeId = NodeId(5);

    const GAP: f32 = 8.;
    const FADE_SECONDS: f64 = 0.25;

    struct Counter {
        count: u32,
        /// Count-label opacity while the new value fades in.
        opacity: f32,
        fade_from: Option<f64>,
        /// Props the backend holds for the count label, the last applied.
        shown: Option<TextProps>,
        cache: MeasureCache,
    }

    impl Counter {
        fn count_props(&self) -> TextProps {
            TextProps { size: Some(48.), tabular_nums: true, opacity: self.opacity, ..TextProps::new(self.count.to_string()) }
        }

        /// What a render produces: creates on the first pass, then only the label whose props
        /// changed.
        fn render(&mut self, tx: &mut Transaction) {
            let count = self.count_props();
            match self.shown.replace(count.clone()) {
                None => {
                    tx.push(Op::Create { node: ROOT, kind: NodeKind::Root, props: props(()) });
                    tx.push(Op::Create {
                        node: SURFACE,
                        kind: NodeKind::View,
                        props: props(ViewProps { background: Some(Color::WindowBackground), ..Default::default() }),
                    });
                    tx.push(Op::Create { node: COUNT, kind: NodeKind::Text, props: props(count) });
                    tx.push(Op::Create {
                        node: CAPTION,
                        kind: NodeKind::Text,
                        props: props(TextProps { color: Color::SecondaryLabel, ..TextProps::new("clicks") }),
                    });
                    tx.push(Op::Create {
                        node: ADD,
                        kind: NodeKind::Widget(WidgetKind::of::<NativeButton>()),
                        props: props(ButtonProps::new("Add one")),
                    });
                    tx.push(Op::Insert { parent: ROOT, node: SURFACE, before: None });
                    for node in [COUNT, CAPTION, ADD] {
                        tx.push(Op::Insert { parent: SURFACE, node, before: None });
                    }
                }
                Some(old) if old != count => {
                    tx.push(Op::Update { node: COUNT, patch: PropsPatch { old: props(old), new: props(count) } });
                }
                Some(_) => {}
            }
        }

        /// `v_flex().size_full().items_center().justify_center().gap_2()` by hand.
        fn layout(&mut self, backend: &mut dyn Backend, size: Size, tx: &mut Transaction) {
            let natural = Constraint { width: AvailableSpace::MaxContent, height: AvailableSpace::MaxContent };
            let column = [COUNT, CAPTION, ADD].map(|n| (n, self.cache.measure(backend, n, natural)));
            let height: f32 = column.iter().map(|(_, s)| s.height).sum::<f32>() + GAP * (column.len() - 1) as f32;
            tx.push(Op::SetFrame { node: SURFACE, frame: Rect::new(0., 0., size.width, size.height) });
            let mut y = (size.height - height) / 2.;
            for (node, s) in column {
                tx.push(Op::SetFrame { node, frame: Rect::new((size.width - s.width) / 2., y, s.width, s.height) });
                y += s.height + GAP;
            }
        }
    }

    fn props<T: std::fmt::Debug + PartialEq + 'static>(p: T) -> Props {
        Rc::new(p)
    }

    pub fn run() {
        let mtm = MainThreadMarker::new().expect("hello runs on the main thread");
        let platform = NdAppKitPlatform::new(mtm);
        let backend = Rc::new(RefCell::new(AppKitBackend::new(mtm)));
        let window = platform.open_window("Hello", 480., 320.);
        let host = Rc::new(WindowHost::new(
            &backend,
            WINDOW,
            &window,
            HostOptions { traffic_lights: Some(Point { x: 20., y: 20. }) },
        ));
        let counter = Rc::new(RefCell::new(Counter {
            count: 0,
            opacity: 1.,
            fade_from: None,
            shown: None,
            cache: MeasureCache::default(),
        }));

        host.on_layout({
            let backend = backend.clone();
            let counter = counter.clone();
            move |size| {
                let mut counter = counter.borrow_mut();
                let mut backend = backend.borrow_mut();
                let mut tx = Transaction::new(WINDOW);
                counter.render(&mut tx);
                // Measuring needs the native nodes, so creates and updates land first.
                tx.commit(&mut *backend, &mut counter.cache);
                let mut tx = Transaction::new(WINDOW);
                let counter = &mut *counter;
                counter.layout(&mut *backend, size, &mut tx);
                tx.commit(&mut *backend, &mut counter.cache);
            }
        });

        host.clock().on_frame({
            let counter = counter.clone();
            let host = Rc::downgrade(&host);
            move |tick| {
                let Some(host) = host.upgrade() else { return };
                let mut counter = counter.borrow_mut();
                let from = *counter.fade_from.get_or_insert(tick.target_timestamp);
                let t = ((tick.target_timestamp - from) / FADE_SECONDS).clamp(0., 1.);
                counter.opacity = (0.35 + 0.65 * t) as f32;
                if t >= 1. {
                    counter.fade_from = None;
                    host.clock().set_animating(false);
                }
                host.request_layout();
            }
        });

        let events = backend.borrow().events();
        events.set_waker({
            let events = events.clone();
            let counter = counter.clone();
            let host = Rc::downgrade(&host);
            move || {
                let Some(host) = host.upgrade() else { return };
                for event in events.drain() {
                    if event.node == ADD && event.kind == NativeEventKind::Click {
                        let mut counter = counter.borrow_mut();
                        counter.count += 1;
                        counter.fade_from = None;
                        host.clock().set_animating(true);
                    }
                }
                host.request_layout();
            }
        });

        platform.show(&window);
        platform.run();
    }
}
