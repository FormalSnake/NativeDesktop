//! Tier 2: the layout and windows family on the AppKit backend and real windows. Every check
//! reads AppKit's own state back: the tab strip's `NSSegmentedControl` segments and selection,
//! the hidden pages' `isHidden`, the `NSSplitView`'s orientation and pane frames, the status
//! page's `NSHostingView`, the scaffold inspector's split item. User input goes through AppKit's
//! own paths: the segmented control's action, `setPosition:ofDividerAtIndex:`, the toolbar's
//! system inspector toggle. Reentrancy: each handler sets its widget's props from inside its own
//! event. `NDSwift` is compiled into a dylib and loaded first, as an app's build script would
//! link it. Prints `ND_LAYOUT_WIDGETS_OK`.

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main("gate_m6::appkit_layout_widgets", || layout::run());
    #[cfg(not(target_os = "macos"))]
    harness::main("gate_m6::appkit_layout_widgets", || {});
}

#[cfg(target_os = "macos")]
mod layout {
    use std::cell::RefCell;
    use std::ffi::{CString, c_char, c_int, c_void};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use nd_backend_appkit::AppHost;
    use nd_backend_appkit::objc2::Message;
    use nd_backend_appkit::objc2::rc::Retained;
    use nd_backend_appkit::objc2::runtime::AnyObject;
    use nd_backend_appkit::objc2_app_kit::{
        NSApplication, NSControl, NSSegmentedControl, NSSplitView, NSSplitViewController,
        NSToolbarInspectorTrackingSeparatorItemIdentifier, NSToolbarToggleInspectorItemIdentifier, NSView,
    };
    use nd_backend_appkit::objc2_foundation::{MainThreadMarker, NSPoint, NSSize};
    use nd_components::split::Axis;
    use nd_components::*;
    use nd_ui::prelude::*;
    use nd_ui::semantics::SemanticAction;
    use nd_ui::{App, NodeId, SemanticNode, WindowId};

    struct Checks(u32);

    impl Checks {
        fn check(&mut self, ok: bool, what: &str) {
            println!("{} {what}", if ok { "ok  " } else { "FAIL" });
            if !ok {
                self.0 += 1;
            }
        }
    }

    #[derive(Default)]
    struct Log {
        tabs: Vec<String>,
        ratios: Vec<f32>,
        collapses: Vec<bool>,
    }

    struct Layout {
        log: Rc<RefCell<Log>>,
        tab: SharedString,
        /// The app refuses tab changes: it keeps `tab` as it is.
        refuse: bool,
        ratio: f32,
    }

    impl Render for Layout {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let page = |id: &str, title: &str, height: f32| {
                tab_page(id.to_string(), title.to_string()).child(div().id(format!("{id}.body")).h(px(height)).w_full())
            };
            v_flex()
                .size_full()
                .gap_2()
                .child(
                    tab_view("tabs")
                        .tab(page("one", "One", 40.))
                        .tab(page("two", "Two", 120.).icon(Icon::Star))
                        .tab(page("three", "Three", 80.))
                        .selected(self.tab.clone())
                        .on_select(cx.listener(|this, e: &TabChanged, _, cx| {
                            this.log.borrow_mut().tabs.push(e.id.to_string());
                            if !this.refuse {
                                this.tab = e.id.clone();
                            }
                            cx.notify();
                        })),
                )
                .child(
                    div().h(px(220.)).w_full().flex_none().child(
                        split("split", Axis::Horizontal)
                            .size_full()
                            .ratio(self.ratio)
                            .first_pane(split_pane().min_size(px(120.)).child(div().id("left").size_full()))
                            .second(div().id("right").size_full())
                            .on_resize(cx.listener(|this, e: &SplitResized, _, cx| {
                                this.log.borrow_mut().ratios.push(e.ratio);
                                this.ratio = e.ratio;
                                cx.notify();
                            })),
                    ),
                )
                .child(div().flex_1().child(
                    status_page("Nothing here").id("status").icon(Icon::Inbox).description("Add a note to start"),
                ))
        }
    }

    struct Shell {
        log: Rc<RefCell<Log>>,
        collapsed: bool,
    }

    impl Render for Shell {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            scaffold().toolbar(toolbar().inspector_toggle()).content(div().size_full()).inspector(
                inspector().collapsed(self.collapsed).on_collapse_change(cx.listener(
                    |this, e: &PaneCollapsed, _, cx| {
                        this.log.borrow_mut().collapses.push(e.collapsed);
                        this.collapsed = e.collapsed;
                        cx.notify();
                    },
                )),
            )
        }
    }

    unsafe extern "C" {
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dlerror() -> *const c_char;
    }

    /// Builds `NDSwift` into a dylib under the target dir (again only when a source is newer)
    /// and loads it, so the backend finds its `@_cdecl` functions the way it does in an app.
    fn load_ndswift() -> Result<(), String> {
        let package = Path::new(env!("CARGO_MANIFEST_DIR")).join("swift/NDSwift/Sources/NDSwift");
        let sources: Vec<PathBuf> = std::fs::read_dir(&package)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "swift"))
            .collect();
        let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("libNDSwiftLayoutTest.dylib");
        let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let stale = match modified(&out) {
            Some(built) => sources.iter().any(|s| modified(s).is_some_and(|m| m > built)),
            None => true,
        };
        if stale {
            let system = |program: &str| {
                let mut c = Command::new(program);
                c.env_remove("SDKROOT").env_remove("DEVELOPER_DIR").env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
                c
            };
            let sdk = system("/usr/bin/xcrun").args(["--sdk", "macosx", "--show-sdk-path"]).output();
            let sdk = String::from_utf8_lossy(&sdk.map_err(|e| e.to_string())?.stdout).trim().to_owned();
            let status = system("/usr/bin/swiftc")
                .args(["-emit-library", "-module-name", "NDSwift", "-target", "arm64-apple-macos14", "-sdk", &sdk])
                .args(&sources)
                .arg("-o")
                .arg(&out)
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err(format!("swiftc failed ({status})"));
            }
        }
        let path = CString::new(out.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        // RTLD_NOW | RTLD_GLOBAL from <dlfcn.h>.
        let handle = unsafe { dlopen(path.as_ptr(), 0x2 | 0x8) };
        if handle.is_null() {
            let err = unsafe { std::ffi::CStr::from_ptr(dlerror()) };
            return Err(err.to_string_lossy().into_owned());
        }
        Ok(())
    }

    fn pump(host: &AppHost, for_: Duration) {
        let end = Instant::now() + for_;
        while Instant::now() < end {
            host.platform().pump(Duration::from_millis(10), Duration::from_millis(40));
            host.update(|app| {
                if app.needs_frame() {
                    app.frame();
                }
            });
        }
    }

    fn node(host: &AppHost, window: WindowId, test_id: &str) -> Option<SemanticNode> {
        host.update(|app: &mut App| {
            let tree = app.semantic_window(window)?;
            tree.root.iter().find(|n| n.test_id.as_deref() == Some(test_id)).cloned()
        })
    }

    fn node_id(host: &AppHost, window: WindowId, test_id: &str) -> NodeId {
        node(host, window, test_id).unwrap_or_else(|| panic!("no node with test id {test_id}")).id
    }

    fn view(host: &AppHost, window: WindowId, test_id: &str) -> Retained<NSView> {
        host.backend().view(node_id(host, window, test_id)).unwrap_or_else(|| panic!("{test_id} has no view"))
    }

    /// The control's action sent to its target, as a click ends.
    fn send_action(control: &NSControl) {
        let mtm = MainThreadMarker::new().expect("main thread");
        let sender: &AnyObject = control;
        let action = control.action().expect("an action");
        unsafe {
            NSApplication::sharedApplication(mtm).sendAction_to_from(action, control.target().as_deref(), Some(sender))
        };
    }

    fn approx(a: f64, b: f64, by: f64) -> bool {
        (a - b).abs() <= by
    }

    pub fn run() -> ! {
        let _lock = ndshell::lock::acquire("nd-backend-appkit layout widgets").expect("machine lock");
        let mut c = Checks(0);
        let loaded = load_ndswift();
        c.check(loaded.is_ok(), &format!("NDSwift builds and loads ({loaded:?})"));

        let log = Rc::new(RefCell::new(Log::default()));
        let (mut main, mut shell) = (None, None);
        let (l1, l2) = (log.clone(), log.clone());
        let host = AppHost::new(|cx| {
            let opts = WindowOptions::main("layout").title("Layout").size(size(px(600.), px(640.)));
            let handle = cx.open_window(opts, move |_, cx| {
                cx.new(|_| Layout { log: l1, tab: "one".into(), refuse: false, ratio: 0.5 })
            });
            main = Some((handle.id(), handle.root().clone()));
            let opts = WindowOptions::main("shell").title("Shell").size(size(px(900.), px(500.)));
            let handle = cx.open_window(opts, move |_, cx| cx.new(|_| Shell { log: l2, collapsed: false }));
            shell = Some((handle.id(), handle.root().clone()));
        });
        let (main, root) = main.expect("layout window");
        let (shell, shell_root) = shell.expect("shell window");
        pump(&host, Duration::from_millis(800));
        let set = |f: &dyn Fn(&mut Layout)| {
            host.update(|app| {
                root.update(app, |v, cx| {
                    f(v);
                    cx.notify();
                })
            });
            pump(&host, Duration::from_millis(200));
        };

        // Tabs: an NSSegmentedControl, selection controlled.
        let strip = view(&host, main, "tabs");
        let seg = strip.downcast_ref::<NSSegmentedControl>().expect("the strip is an NSSegmentedControl").retain();
        let labels: Vec<String> =
            (0..seg.segmentCount()).filter_map(|i| seg.labelForSegment(i).map(|s| s.to_string())).collect();
        c.check(labels == ["One", "Two", "Three"], &format!("three segments labelled by title ({labels:?})"));
        c.check(seg.imageForSegment(1).is_some(), "the icon tab shows its SF Symbol");
        c.check(seg.selectedSegment() == 0, "the selected tab's segment is selected");
        let size = seg.fittingSize();
        let frame = seg.frame().size;
        c.check(
            approx(frame.width, size.width, 1.) && approx(frame.height, size.height, 1.),
            &format!("the strip is laid out at its fitting size ({frame:?} vs {size:?})"),
        );
        let pages = ["one", "two", "three"].map(|p| view(&host, main, p));
        let hidden = || pages.iter().map(|p| p.isHidden()).collect::<Vec<_>>();
        c.check(hidden() == [false, true, true], &format!("pages not selected are hidden NSViews ({:?})", hidden()));
        let heights = || pages.iter().map(|p| p.frame().size.height).collect::<Vec<_>>();
        let first_heights = heights();
        c.check(
            first_heights.iter().all(|h| approx(*h, 120., 0.5)),
            &format!("every page takes the tallest page's height ({first_heights:?})"),
        );
        let stack = unsafe { pages[0].superview() }.expect("pages share a stack");
        let center = NSPoint::new(stack.bounds().size.width / 2., first_heights[0] / 2.);
        let center = stack.convertPoint_toView(center, unsafe { stack.superview() }.as_deref());
        let hit = unsafe { stack.superview() }.and_then(|s| s.hitTest(center));
        let in_hidden = |v: &NSView| pages[1..].iter().any(|p| v.isDescendantOf(p));
        c.check(hit.as_deref().is_some_and(|h| !in_hidden(h)), "a click over the pages lands in the shown page only");

        seg.setSelectedSegment(1);
        send_action(&seg);
        pump(&host, Duration::from_millis(250));
        c.check(log.borrow().tabs == ["two"], &format!("a click emits TabChanged once ({:?})", log.borrow().tabs));
        c.check(seg.selectedSegment() == 1, "the app's selection lands on the clicked segment");
        c.check(hidden() == [true, false, true], "the selected page shows, the others hide");
        c.check(heights() == first_heights, &format!("switching keeps the pages' height ({:?})", heights()));
        let probed = node(&host, main, "tabs");
        c.check(
            probed.as_ref().is_some_and(|n| n.value.as_deref() == Some("Two") && n.enabled),
            "probe reports the selected title and enabled",
        );

        set(&|v| v.refuse = true);
        seg.setSelectedSegment(2);
        send_action(&seg);
        pump(&host, Duration::from_millis(250));
        c.check(log.borrow().tabs == ["two", "three"], "a refused click emits once");
        c.check(seg.selectedSegment() == 1, "an app that refuses the change gets its old segment back");
        set(&|v| v.refuse = false);

        let id = node_id(&host, main, "tabs");
        let done = host.update(|app| app.perform(id, &SemanticAction::SetValue("Three".into())));
        pump(&host, Duration::from_millis(250));
        c.check(done.is_ok(), &format!("setValue on the strip is taken ({done:?})"));
        c.check(
            log.borrow().tabs.len() == 3 && seg.selectedSegment() == 2,
            "setValue selects through the control's action",
        );

        // Split: an NSSplitView whose panes the delegate places by the ratio.
        let split = view(&host, main, "split");
        let split = split.downcast_ref::<NSSplitView>().expect("the split is an NSSplitView").retain();
        c.check(split.isVertical(), "a horizontal split is a vertical NSSplitView (side by side)");
        let avail = || split.bounds().size.width - split.dividerThickness();
        let first = || split.subviews().objectAtIndex(0).frame();
        let second = || split.subviews().objectAtIndex(1).frame();
        c.check(split.subviews().count() == 2, "both panes are arranged subviews");
        c.check(
            approx(first().size.width, 0.5 * avail(), 1.),
            &format!("the first pane's width follows the ratio ({} of {})", first().size.width, avail()),
        );
        c.check(
            approx(second().origin.x, first().size.width + split.dividerThickness(), 1.),
            "the second pane starts after the divider",
        );
        let left = view(&host, main, "left");
        c.check(
            approx(left.frame().size.width, first().size.width, 1.) && left.frame().size.height > 0.,
            &format!("the pane's content fills the frame the split view gave it ({:?})", left.frame().size),
        );

        let placed = host.backend().split_placements(node_id(&host, main, "split")).expect("a split");
        split.setPosition_ofDividerAtIndex(0.3 * avail(), 0);
        pump(&host, Duration::from_millis(250));
        let ratios = log.borrow().ratios.clone();
        c.check(
            ratios.len() == 1 && approx(f64::from(ratios[0]), 0.3, 0.01),
            &format!("moving the divider emits SplitResized once ({ratios:?})"),
        );
        let after = host.backend().split_placements(node_id(&host, main, "split"));
        c.check(after == Some(placed), "feeding the reported ratio back places nothing (echo)");
        c.check(approx(left.frame().size.width, first().size.width, 1.), "the pane's content follows the new frame");

        split.setPosition_ofDividerAtIndex(10., 0);
        pump(&host, Duration::from_millis(250));
        let ratios = log.borrow().ratios.clone();
        c.check(
            approx(first().size.width, 120., 0.5) && ratios.len() == 2,
            &format!("a divider move stops at the pane's min size and reports once ({ratios:?})"),
        );

        set(&|v| v.ratio = 0.02);
        c.check(approx(first().size.width, 120., 0.5), &format!("the min size clamps ({})", first().size.width));
        c.check(log.borrow().ratios.len() == 2, "a clamped placement reports nothing");
        set(&|v| v.ratio = 0.4);

        let window = host.ns_window(main).expect("window");
        let before = log.borrow().ratios.len();
        window.setContentSize(NSSize::new(760., 640.));
        pump(&host, Duration::from_millis(300));
        c.check(log.borrow().ratios.len() == before, "a window resize reports nothing");
        c.check(
            approx(first().size.width / avail(), 0.4, 0.01),
            &format!("a window resize keeps the ratio ({:.3})", first().size.width / avail()),
        );

        // Status page: SwiftUI's ContentUnavailableView in an NSHostingView.
        let status = view(&host, main, "status");
        let class = status.class().name().to_string_lossy().into_owned();
        c.check(class.contains("NSHostingView"), &format!("the status page is an NSHostingView ({class})"));
        let s = status.frame().size;
        c.check(s.width > 0. && s.height > 0., &format!("it measures and lays out non-zero ({s:?})"));

        // Inspector: the toolbar's system toggle collapses it, reported once.
        let shell_window = host.ns_window(shell).expect("shell window");
        shell_window.makeKeyAndOrderFront(None);
        pump(&host, Duration::from_millis(500));
        let toolbar = shell_window.toolbar().expect("a toolbar");
        let ids: Vec<String> = toolbar.items().iter().map(|i| i.itemIdentifier().to_string()).collect();
        let tracking = unsafe { NSToolbarInspectorTrackingSeparatorItemIdentifier }.to_string();
        let toggle = unsafe { NSToolbarToggleInspectorItemIdentifier }.to_string();
        let (t, g) = (ids.iter().position(|i| *i == tracking), ids.iter().position(|i| *i == toggle));
        c.check(
            matches!((t, g), (Some(t), Some(g)) if t < g),
            &format!("the inspector tracking separator sits ahead of the inspector toggle ({ids:?})"),
        );
        let controller = shell_window
            .contentViewController()
            .and_then(|c| c.downcast::<NSSplitViewController>().ok())
            .expect("the scaffold's split view controller");
        let item = controller.splitViewItems().lastObject().expect("the inspector item");
        let item_toggle = toolbar.items().iter().find(|i| i.itemIdentifier().to_string() == toggle);
        if let Some(item_toggle) = item_toggle {
            let mtm = MainThreadMarker::new().expect("main thread");
            let sender: &AnyObject = &item_toggle;
            unsafe {
                NSApplication::sharedApplication(mtm).sendAction_to_from(
                    item_toggle.action().expect("toggle action"),
                    item_toggle.target().as_deref(),
                    Some(sender),
                )
            };
        }
        pump(&host, Duration::from_millis(1000));
        c.check(item.isCollapsed(), "the toolbar toggle collapses the inspector");
        let collapses = log.borrow().collapses.clone();
        c.check(collapses == [true], &format!("the user collapse is reported once ({collapses:?})"));
        host.update(|app| {
            shell_root.update(app, |v, cx| {
                v.collapsed = false;
                cx.notify();
            })
        });
        pump(&host, Duration::from_millis(1000));
        c.check(!item.isCollapsed(), "a controlled expand opens the inspector");
        let collapses = log.borrow().collapses.clone();
        c.check(collapses == [true], &format!("a controlled change echoes nothing ({collapses:?})"));

        shell_window.close();
        window.close();
        if c.0 == 0 {
            println!("ND_LAYOUT_WIDGETS_OK");
            std::process::exit(0);
        }
        println!("ND_LAYOUT_WIDGETS_FAILED {} check(s)", c.0);
        std::process::exit(1);
    }
}
