//! Tier 2: windows on [`AppHost`] (ARCHITECTURE.md 5.5). Windows of one `tab_group` share one
//! `NSWindowTabGroup`, the tab bar's `+` (`newWindowForTab:` through `NSApp.sendAction`) and a
//! user close (`performClose:`) reach the app on a later turn, a held close keeps the window, a
//! view moved to another window keeps its WKWebView and page, and alerts and the about panel are
//! AppKit's own. Handlers open and close windows from inside themselves. Prints
//! `ND_LAYOUT_WINDOWS_OK`.

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main("gate_m6::appkit_layout_windows", || windows::run());
    #[cfg(not(target_os = "macos"))]
    harness::main("gate_m6::appkit_layout_windows", || {});
}

#[cfg(target_os = "macos")]
mod windows {
    use std::cell::{Cell, RefCell};
    use std::ptr::NonNull;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use block2::RcBlock;
    use nd_backend_appkit::objc2::rc::Retained;
    use nd_backend_appkit::objc2::{msg_send, sel};
    use nd_backend_appkit::objc2_app_kit::{NSApplication, NSPanel, NSTextField, NSView, NSWindow};
    use nd_backend_appkit::objc2_foundation::{MainThreadMarker, NSRunLoop, NSRunLoopCommonModes, NSString, NSTimer};
    use nd_backend_appkit::{AppHost, AppKitChrome};
    use nd_components::services::{self, Chrome};
    use nd_components::{AboutPanel, Alert};
    use nd_ui::backend::WindowCommand;
    use nd_ui::prelude::*;
    use nd_ui::surface::{SurfaceSource, native_surface};
    use nd_ui::{App, WindowId};

    /// Gives every load of the page a new title, so a reload shows.
    const PAGE: &str = "<html><body><script>document.title='load-'+Math.random();</script></body></html>";

    struct Page;

    impl Render for Page {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(native_surface("web", SurfaceSource::Html(PAGE.into())).w(px(300.)).h(px(200.)))
        }
    }

    struct Shelf {
        page: Option<Entity<Page>>,
    }

    impl Render for Shelf {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            v_flex().size_full().child(text("shelf")).children(self.page.clone())
        }
    }

    #[derive(Default)]
    struct Log {
        new_tabs: u32,
        held: u32,
        opened: Vec<WindowId>,
        re_asked: u32,
        spawned: Option<WindowId>,
    }

    type Shared = Rc<RefCell<Log>>;

    fn shelf(cx: &mut App) -> Entity<Shelf> {
        cx.new(|_| Shelf { page: None })
    }

    fn options(title: &str) -> WindowOptions {
        WindowOptions::main(title.to_owned()).title(title.to_owned()).size(size(px(420.), px(320.)))
    }

    /// A tab of group "t": its `+` opens another one, and its close is held.
    fn open_tab(cx: &mut App, log: &Shared, title: &str) -> WindowId {
        let log = log.clone();
        cx.open_window(options(title).tab_group("t"), move |window, cx| {
            let on_new = log.clone();
            window.on_new_tab(move |_, cx| {
                on_new.borrow_mut().new_tabs += 1;
                let id = open_tab(cx, &on_new, "t-new");
                on_new.borrow_mut().opened.push(id);
            });
            let on_close = log.clone();
            window.on_should_close(move |_, _| {
                on_close.borrow_mut().held += 1;
                false
            });
            shelf(cx)
        })
        .id()
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

    fn pump(host: &AppHost) {
        for _ in 0..8 {
            host.platform().pump(Duration::from_millis(2), Duration::from_millis(30));
            host.update(|app| {
                if app.needs_frame() {
                    app.frame();
                }
            });
        }
    }

    fn pump_until(host: &AppHost, secs: f64, mut done: impl FnMut(&AppHost) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs_f64(secs);
        while Instant::now() < deadline {
            if done(host) {
                return true;
            }
            pump(host);
        }
        done(host)
    }

    fn class_name(view: &NSView) -> String {
        view.class().name().to_string_lossy().into_owned()
    }

    fn find_view(view: &NSView, pred: &dyn Fn(&NSView) -> bool) -> Option<Retained<NSView>> {
        for sub in view.subviews().iter() {
            if pred(&sub) {
                return Some(sub);
            }
            if let Some(found) = find_view(&sub, pred) {
                return Some(found);
            }
        }
        None
    }

    fn web_title(view: &NSView) -> Option<String> {
        let title: Option<Retained<NSString>> = unsafe { msg_send![view, title] };
        title.map(|t| t.to_string()).filter(|t| !t.is_empty())
    }

    fn same(a: &NSWindow, b: Option<&NSWindow>) -> bool {
        b.is_some_and(|b| std::ptr::eq(a, b))
    }

    /// Runs `f` once after `seconds`, in common modes, so it fires inside a modal session too.
    fn run_after(seconds: f64, f: impl Fn() + 'static) -> Retained<NSTimer> {
        let block = RcBlock::new(move |_timer: NonNull<NSTimer>| f());
        let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(seconds, false, &block) };
        unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
        timer
    }

    pub fn run() -> ! {
        let mtm = MainThreadMarker::new().expect("main thread");
        let log: Shared = Rc::default();
        let mut ids = Vec::new();
        let mut page = None;
        let (mut a, mut b) = (None, None);
        let re_log = log.clone();
        let host = AppHost::new(|cx| {
            cx.set_global(Chrome(Rc::new(AppKitChrome::new(
                cx.backend_as::<nd_backend_appkit::AppKitBackend>().expect("appkit").clone(),
            ))));
            for title in ["t1", "t2", "t3"] {
                ids.push(open_tab(cx, &log, title));
            }
            ids.push(cx.open_window(options("u1").tab_group("u"), |_, cx| shelf(cx)).id());
            let first = cx.open_window(options("a"), |_, cx| {
                let p = cx.new(|_| Page);
                page = Some(p.clone());
                cx.new(|_| Shelf { page: Some(p) })
            });
            a = Some(first);
            b = Some(cx.open_window(options("b"), |_, cx| shelf(cx)));
            ids.push(cx.open_window(options("plain"), |_, cx| shelf(cx)).id());
            let spare = cx.open_window(options("spare"), |_, cx| shelf(cx)).id();
            ids.push(spare);
            let re = re_log.clone();
            ids.push(
                cx.open_window(options("reentrant"), move |window, cx| {
                    // Opens one window and closes another from inside the close handler.
                    window.on_should_close(move |_, cx| {
                        re.borrow_mut().re_asked += 1;
                        let id = cx.open_window(options("spawned"), |_, cx| shelf(cx)).id();
                        re.borrow_mut().spawned = Some(id);
                        cx.close_window(spare);
                        true
                    });
                    shelf(cx)
                })
                .id(),
            );
        });
        let (a, b, page) = (a.unwrap(), b.unwrap(), page.unwrap());
        let [t1, t2, t3, u1, plain, spare, re] = ids[..] else { panic!("seven windows") };
        let ns = |id: WindowId| host.ns_window(id).expect("open");
        let backend = host.backend().clone();
        pump(&host);
        let mut c = Checks(0);

        // Tab groups.
        c.check(backend.tab_group_windows(t1) == [t1, t2, t3], "t1, t2, t3 share one tab group in open order");
        let group = ns(t1).tabGroup();
        c.check(
            group.as_ref().is_some_and(|g| [t2, t3].iter().all(|w| ns(*w).tabGroup().as_ref() == Some(g))),
            "it is one NSWindowTabGroup",
        );
        c.check(backend.tab_group_windows(u1) == [u1], "u1 stays in its own group");
        c.check(*ns(u1).tabbingIdentifier() != *ns(t1).tabbingIdentifier(), "groups carry distinct identifiers");
        c.check(backend.tab_group_windows(plain) == [plain], "a plain window is untabbed");

        // A tab retitles itself, from outside and from inside one of its own handlers.
        host.update(|app| app.set_window_title(t1, "Renamed"));
        c.check(ns(t1).title().to_string() == "Renamed", "set_window_title sets the tab's NSWindow title");
        c.check(
            host.update(|app| app.window(t1).map(|w| w.title().to_string())).as_deref() == Some("Renamed"),
            "and the Window's",
        );

        // Activate selects a tab.
        host.update(|app| app.window_command(t2, WindowCommand::Activate));
        pump(&host);
        c.check(backend.selected_tab(t1) == Some(t2), "Activate selects t2's tab");
        host.update(|app| app.activate_window(t3));
        let key = pump_until(&host, 3., |_| ns(t3).isKeyWindow());
        c.check(backend.selected_tab(t1) == Some(t3), "activate_window selects t3's tab");

        // The tab bar's `+`.
        if key {
            let app = NSApplication::sharedApplication(mtm);
            let sent = unsafe { app.sendAction_to_from(sel!(newWindowForTab:), None, None) };
            c.check(sent, "newWindowForTab: finds a target in the key window's responder chain");
            let opened =
                pump_until(&host, 3., |h| log.borrow().opened.first().is_some_and(|id| h.ns_window(*id).is_some()));
            c.check(opened && log.borrow().new_tabs == 1, "on_new_tab ran once and its window opened");
            if let Some(new) = log.borrow().opened.first().copied() {
                pump(&host);
                c.check(backend.tab_group_windows(t1).contains(&new), "the new window joined group t");
                c.check(backend.tab_group_windows(t1).len() == 4, "group t has four tabs");
            }
        } else {
            c.check(false, "t3 became key (rerun alone if another test held the key window)");
        }

        // A held close.
        let ns_t2 = ns(t2);
        host.update(|app| app.activate_window(t2));
        pump(&host);
        ns_t2.performClose(None);
        pump_until(&host, 2., |_| log.borrow().held > 0);
        pump(&host);
        c.check(log.borrow().held == 1, "performClose: asked the handler once");
        c.check(host.update(|app| app.window(t2).is_some()) && ns_t2.isVisible(), "the held window stays open");
        c.check(backend.tab_group_windows(t1).contains(&t2), "and stays in its group");
        let tabs = backend.tab_group_windows(t1).len();
        host.update(|app| app.close_window(t2));
        pump(&host);
        c.check(!ns_t2.isVisible(), "close_window closes the NSWindow");
        c.check(!backend.tab_group_windows(t1).contains(&t2), "and it left the tab group");
        c.check(ns(t1).tabGroup().is_some_and(|g| g.windows().count() == tabs - 1), "AppKit's group has one tab fewer");

        // A plain close.
        let ns_plain = ns(plain);
        ns_plain.performClose(None);
        pump_until(&host, 2., |h| h.update(|app| app.window(plain).is_none()));
        pump(&host);
        c.check(host.update(|app| app.window(plain).is_none()), "a plain window closes on performClose:");
        c.check(!ns_plain.isVisible(), "and its NSWindow is gone");

        // Reentrant close handler.
        ns(re).performClose(None);
        pump_until(&host, 2., |_| log.borrow().spawned.is_some());
        pump(&host);
        let spawned = log.borrow().spawned;
        c.check(log.borrow().re_asked == 1, "the reentrant handler ran once");
        c.check(
            host.update(|app| app.window(re).is_none() && app.window(spare).is_none()),
            "it closed itself and the window it closed from inside",
        );
        c.check(spawned.is_some_and(|s| host.ns_window(s).is_some_and(|w| w.isVisible())), "its new window opened");

        // A view with a web view moves between windows.
        let page_node = host.update(|app| app.view_node(page.entity_id())).expect("page mounted");
        let web = backend
            .view(page_node)
            .and_then(|v| find_view(&v, &|v| class_name(v).contains("WKWebView")))
            .expect("the page holds a WKWebView");
        let loaded = pump_until(&host, 5., |_| web_title(&web).is_some());
        let title = web_title(&web);
        c.check(loaded, &format!("the page loaded ({title:?})"));
        let web_node = backend.node_for_view(&*web).expect("the web view is a node");
        c.check(backend.node_window(web_node) == Some(a.id()), "it starts in window a");
        host.update(|app| {
            let moved = a.root().update(app, |s, cx| {
                cx.notify();
                s.page.take()
            });
            b.root().update(app, |s, cx| {
                s.page = moved;
                cx.notify();
            });
            app.frame();
        });
        pump_until(&host, 1., |_| false);
        let after = backend.view(page_node).and_then(|v| find_view(&v, &|v| class_name(v).contains("WKWebView")));
        c.check(after.as_ref().is_some_and(|v| std::ptr::eq(&**v, &*web)), "the same WKWebView, not a new one");
        c.check(same(&ns(b.id()), web.window().as_deref()), "it is in window b's view tree");
        c.check(web_title(&web) == title, "the page was not reloaded");
        c.check(backend.node_window(web_node) == Some(b.id()), "node_window follows the move");
        c.check(backend.node_window(page_node) == Some(b.id()), "for the page's root too");

        // Alerts: a modal one, then two sheets on one window.
        let answers: Rc<RefCell<Vec<(&'static str, usize)>>> = Rc::default();
        let saw_modal = Rc::new(Cell::new(false));
        let stopper = {
            let saw = saw_modal.clone();
            run_after(0.4, move || {
                let app = NSApplication::sharedApplication(mtm);
                saw.set(app.modalWindow().is_some());
                app.stopModalWithCode(1001);
            })
        };
        let record = |name: &'static str| {
            let answers = answers.clone();
            move |r: nd_components::AlertResponse, _: &mut App| answers.borrow_mut().push((name, r.0))
        };
        host.update(|app| {
            services::alert(app, a.id(), Alert::new("Modal").button("Yes").cancel("No").modal(), record("modal"))
        });
        pump_until(&host, 3., |_| !answers.borrow().is_empty());
        stopper.invalidate();
        c.check(saw_modal.get(), "the modal alert ran app-modal");
        c.check(*answers.borrow() == [("modal", 1)], &format!("it answered once: {:?}", answers.borrow()));

        let ns_a = ns(a.id());
        host.update(|app| {
            services::alert(app, a.id(), Alert::new("First").button("OK"), record("first"));
            services::alert(app, a.id(), Alert::new("Second").button("OK").cancel("Cancel"), record("second"));
        });
        let first = pump_until(&host, 3., |_| ns_a.attachedSheet().is_some());
        let sheet = ns_a.attachedSheet();
        c.check(first, "the first sheet alert is attached to window a");
        c.check(answers.borrow().len() == 1, "the second waits for it");
        if let Some(sheet) = &sheet {
            ns_a.endSheet_returnCode(sheet, 1000);
        }
        let second = pump_until(&host, 3., |_| {
            ns_a.attachedSheet().is_some_and(|s| sheet.as_ref().is_none_or(|old| !std::ptr::eq(&*s, &**old)))
        });
        c.check(second, "the second sheet shows once the first is answered");
        if let Some(sheet) = ns_a.attachedSheet() {
            ns_a.endSheet_returnCode(&sheet, 1001);
        }
        pump_until(&host, 2., |_| answers.borrow().len() == 3);
        c.check(
            *answers.borrow() == [("modal", 1), ("first", 0), ("second", 1)],
            &format!("each sheet answered once: {:?}", answers.borrow()),
        );

        // The about panel.
        let name = "Layout Windows Gate";
        host.update(|app| {
            services::about(
                app,
                Some(a.id()),
                AboutPanel::new()
                    .name(name)
                    .version("1.2")
                    .build("34")
                    .credits("Made in a test")
                    .website("https://example.com"),
            )
        });
        let find_panel = || {
            NSApplication::sharedApplication(mtm).windows().iter().find(|w| {
                w.isVisible()
                    && w.downcast_ref::<NSPanel>().is_some()
                    && w.contentView().is_some_and(|v| {
                        find_view(&v, &|v| {
                            v.downcast_ref::<NSTextField>().is_some_and(|f| f.stringValue().to_string() == name)
                        })
                        .is_some()
                    })
            })
        };
        let shown = pump_until(&host, 3., |_| find_panel().is_some());
        c.check(shown, "the standard about panel shows the app name");
        if let Some(panel) = find_panel() {
            panel.close();
        }

        if c.0 == 0 {
            println!("ND_LAYOUT_WINDOWS_OK");
        } else {
            println!("{} checks failed", c.0);
        }
        std::process::exit(i32::from(c.0 != 0));
    }
}
