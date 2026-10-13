//! Tier 2: the macOS chrome intents on a real window, each checked against the system component
//! it must be (ARCHITECTURE.md 3.6, 3.10): `scaffold()` an `NSSplitViewController` with sidebar
//! and inspector items whose panes lay their content out at the size the split view gives,
//! `toolbar()` an `NSToolbar` with system items, `glass()` an `NSGlassEffectView` on 26+, the
//! menu bar an `NSMenu` whose key equivalents come from the bindings, alerts and pickers sheets
//! on the window, `open_sheet` a window-modal sheet, the settings window preference-styled, and
//! embedded traffic lights riding their slot. `ND_CHROME_SHOT=<dir>` also screenshots the
//! windows there. Prints `ND_CHROME_OK`.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let name = "gate_m3::appkit_chrome_intents";
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
    chrome::run();
}

#[cfg(target_os = "macos")]
mod chrome {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use nd::appkit::AppHost;
    use nd::appkit::objc2::rc::Retained;
    use nd::appkit::objc2::{Message, msg_send};
    use nd::appkit::objc2_app_kit::{
        NSApplication, NSOutlineView, NSSplitView, NSSplitViewController, NSSplitViewItemBehavior, NSTableViewStyle,
        NSView, NSWindow, NSWindowButton, NSWindowToolbarStyle,
    };
    use nd::appkit::objc2_foundation::{MainThreadMarker, NSIndexSet};
    use nd::prelude::*;

    actions!(chrome_test, [NewThing]);

    struct Shell {
        folder: SharedString,
    }

    impl Render for Shell {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            scaffold()
                .toolbar(
                    toolbar()
                        .title("Chrome")
                        .sidebar_toggle()
                        .item(icon_button("new", Icon::SquarePen).tooltip("New").dispatch(NewThing))
                        .flexible_space()
                        .item(button("share", "Share"))
                        .inspector_toggle(),
                )
                .sidebar(
                    sidebar().min_w(px(180.)).child(
                        source_list("folders")
                            .section(
                                "Library",
                                [
                                    source_item("inbox", "Inbox").icon(Icon::Inbox).badge("3"),
                                    source_item("archive", "Archive").icon(Icon::Archive),
                                ],
                            )
                            .section("Tags", [source_item("starred", "Starred").icon(Icon::Star)])
                            .selected(self.folder.clone())
                            .on_select(cx.listener(|this, e: &SourceSelected, _, cx| {
                                this.folder = e.id.clone();
                                cx.notify();
                            })),
                    ),
                )
                .content(
                    v_flex()
                        .size_full()
                        .p_4()
                        .gap_3()
                        .child(text("Content").id("content-label"))
                        .child(glass().id("glass").rounded_lg().p_3().child(text("On glass"))),
                )
                .inspector(inspector().min_w(px(200.)).child(v_flex().p_3().child(text("Inspector"))))
        }
    }

    struct Row;

    impl Render for Row {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            // A slot pushed right by a 60 pt item: the real buttons must follow it.
            v_flex()
                .size_full()
                .child(h_flex().pt(px(12.)).child(div().w(px(60.))).child(window_controls().embedded()))
                .child(div().flex_1())
        }
    }

    struct Sheet;

    impl Render for Sheet {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            v_flex().size_full().p_4().child(text("A sheet"))
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

    fn find<T: Message>(view: &NSView, f: &mut dyn FnMut(&NSView) -> bool) -> Option<Retained<NSView>> {
        if f(view) {
            return Some(view.retain());
        }
        view.subviews().iter().find_map(|s| find::<T>(&s, f))
    }

    fn class_is(view: &NSView, name: &str) -> bool {
        view.class().name().to_str().unwrap_or("").contains(name)
    }

    fn label_view(root: &NSView, text: &str) -> Option<Retained<NSView>> {
        find::<NSView>(root, &mut |v| {
            class_is(v, "NdTextView")
                && nd::appkit::objc2_app_kit::NSAccessibility::accessibilityValue(v)
                    .and_then(|o| o.downcast::<nd::appkit::objc2_foundation::NSString>().ok())
                    .is_some_and(|s| s.to_string() == text)
        })
    }

    fn shot(window: &NSWindow, name: &str) {
        let Some(dir) = std::env::var_os("ND_CHROME_SHOT") else { return };
        let path = std::path::Path::new(&dir).join(format!("{name}.png"));
        let _ = std::process::Command::new("/usr/sbin/screencapture")
            .args(["-x", "-o", &format!("-l{}", window.windowNumber())])
            .arg(&path)
            .status();
        println!("     screenshot {}", path.display());
    }

    pub fn run() -> ! {
        let dir = std::env::temp_dir().join(format!("nd-chrome-{}", std::process::id()));
        unsafe {
            std::env::set_var("ND_APP_DATA_DIR", &dir);
            std::env::set_var("ND_THEME_TOML", "");
            std::env::set_var("ND_THEME_JSON", "");
        }
        let mut main = None;
        let host = Application::new(nd::manifest!()).host(|cx| {
            cx.bind_keys([KeyBinding::new("secondary-n", NewThing, None)]);
            set_menus(cx, [Menu::new("File").items([MenuItem::action("New Thing", NewThing)])]);
            main =
                Some(cx.open_window(
                    WindowOptions::main("chrome").title("Chrome").size(size(px(900.), px(560.))),
                    |_, cx| cx.new(|_| Shell { folder: "inbox".into() }),
                ));
        });
        let main = main.expect("main window");
        pump(&host, Duration::from_millis(800));
        let mtm = MainThreadMarker::new().expect("main thread");
        let window = host.ns_window(main.id()).expect("window");
        let content = window.contentView().expect("content view");
        let mut c = Checks(0);

        // Scaffold: the split view controller and its items.
        let split = find::<NSSplitView>(&content, &mut |v| v.downcast_ref::<NSSplitView>().is_some());
        c.check(split.is_some(), "scaffold() is an NSSplitView in the window");
        let behaviors: Vec<NSSplitViewItemBehavior> = split
            .as_ref()
            .and_then(|s| {
                let delegate: Option<Retained<nd::appkit::objc2::runtime::AnyObject>> =
                    unsafe { msg_send![&**s, delegate] };
                delegate?.downcast::<NSSplitViewController>().ok()
            })
            .map(|vc| vc.splitViewItems().iter().map(|i| i.behavior()).collect())
            .unwrap_or_default();
        c.check(
            behaviors
                == [
                    NSSplitViewItemBehavior::Sidebar,
                    NSSplitViewItemBehavior::Default,
                    NSSplitViewItemBehavior::Inspector,
                ],
            &format!("its NSSplitViewController has sidebar, content and inspector items ({behaviors:?})"),
        );
        let content_label = label_view(&content, "Content");
        let in_split = content_label.as_ref().is_some_and(|l| {
            let r = l.convertRect_toView(l.bounds(), split.as_deref());
            r.size.width > 0. && r.origin.x > 150.
        });
        c.check(in_split, "content lays out inside the content pane, right of the sidebar");
        // Source list.
        let outline = find::<NSView>(&content, &mut |v| v.downcast_ref::<NSOutlineView>().is_some())
            .and_then(|v| v.downcast::<NSOutlineView>().ok());
        c.check(
            outline.as_ref().is_some_and(|o| o.style() == NSTableViewStyle::SourceList),
            "source_list() is an NSOutlineView in the source list style",
        );
        if let Some(outline) = &outline {
            c.check(
                outline.numberOfRows() == 5,
                &format!("two group rows and three items ({} rows)", outline.numberOfRows()),
            );
            c.check(outline.selectedRow() == 1, "the app's selection is the outline's");
            // A user selection: AppKit's own selection change, reported once to the app.
            outline.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(4), false);
            pump(&host, Duration::from_millis(200));
            let folder = host.update(|cx| main.root().read(cx).folder.clone());
            c.check(folder.as_ref() == "starred", &format!("a selection reaches the app as SourceSelected ({folder})"));
            c.check(outline.selectedRow() == 4, "the app's echo of the selection sets nothing back");
        }
        c.check(
            label_view(&content, "Inspector").is_some_and(|l| l.frame().size.width > 0.),
            "inspector content is laid out",
        );

        // Toolbar.
        let ids: Vec<String> = window
            .toolbar()
            .map(|t| t.items().iter().map(|i| i.itemIdentifier().to_string()).collect())
            .unwrap_or_default();
        let has = |id: &str| ids.iter().any(|i| i == id);
        c.check(has("NSToolbarToggleSidebarItem"), &format!("toolbar has the system sidebar toggle ({ids:?})"));
        c.check(has("NSToolbarSidebarTrackingSeparatorItemIdentifier"), "toolbar tracks the sidebar divider");
        c.check(has("NSToolbarFlexibleSpaceItem"), "toolbar has the system flexible space");
        c.check(has("NSToolbarToggleInspectorItem"), "toolbar has the system inspector toggle");
        let image_item =
            window.toolbar().and_then(|t| t.items().iter().find(|i| i.image().is_some() && i.view().is_none()));
        c.check(image_item.is_some(), "an icon button is the toolbar's own image item");
        c.check(window.title().to_string() == "Chrome", "toolbar title is the window title");

        // Glass.
        let glass =
            find::<NSView>(&content, &mut |v| class_is(v, "NSGlassEffectView") || class_is(v, "NSVisualEffectView"));
        let want = if nd::appkit::objc2::runtime::AnyClass::get(c"NSGlassEffectView").is_some() {
            "NSGlassEffectView"
        } else {
            "NSVisualEffectView"
        };
        c.check(glass.as_ref().is_some_and(|g| class_is(g, want)), &format!("glass() is {want}"));
        shot(&window, "chrome-main");

        // Menu bar.
        let bar = NSApplication::sharedApplication(mtm).mainMenu();
        let titles: Vec<String> =
            bar.as_ref().map(|m| m.itemArray().iter().map(|i| i.title().to_string()).collect()).unwrap_or_default();
        c.check(
            titles.len() >= 4 && titles[1..].starts_with(&["File".into(), "Edit".into()]),
            &format!("menu bar has app, File, Edit, Window ({titles:?})"),
        );
        let new_item = bar
            .as_ref()
            .and_then(|m| m.itemWithTitle(&nd::appkit::objc2_foundation::NSString::from_str("File")))
            .and_then(|f| f.submenu())
            .and_then(|s| s.itemWithTitle(&nd::appkit::objc2_foundation::NSString::from_str("New Thing")));
        c.check(
            new_item.as_ref().is_some_and(|i| i.keyEquivalent().to_string() == "n"),
            "File > New Thing shows the secondary-n binding as cmd-n",
        );

        // Alert as a sheet.
        let answer = Rc::new(RefCell::new(None));
        let got = answer.clone();
        host.update(|cx| {
            nd::chrome::alert(
                cx,
                main.id(),
                Alert::new("Delete?").destructive("Delete").cancel("Cancel"),
                move |r, _| *got.borrow_mut() = Some(r),
            )
        });
        pump(&host, Duration::from_millis(300));
        let sheet = window.attachedSheet();
        c.check(sheet.is_some(), "alert() is a sheet on the window");
        if let Some(sheet) = sheet {
            shot(&sheet, "chrome-alert");
            window.endSheet_returnCode(&sheet, 1001);
        }
        pump(&host, Duration::from_millis(200));
        c.check(*answer.borrow() == Some(AlertResponse(1)), "the alert's answer reaches the app by button index");

        // Open panel as a sheet.
        let picked = Rc::new(RefCell::new(Some(Vec::new())));
        let got = picked.clone();
        host.update(|cx| nd::chrome::open_dialog(cx, main.id(), OpenDialog::file(), move |p, _| *got.borrow_mut() = p));
        pump(&host, Duration::from_millis(600));
        let panel = window.attachedSheet();
        c.check(panel.is_some(), "open_dialog() is an NSOpenPanel sheet");
        if let Some(panel) = panel {
            window.endSheet_returnCode(&panel, 0);
        }
        pump(&host, Duration::from_millis(300));
        c.check(picked.borrow().is_none(), "a cancelled picker answers None");

        // A sheet holding an nd view.
        let sheet = host.update(|cx| {
            cx.open_window(
                WindowOptions::main("chrome.sheet")
                    .kind(WindowKind::Sheet { parent: main.id() })
                    .size(size(px(320.), px(200.))),
                |_, cx| cx.new(|_| Sheet),
            )
        });
        pump(&host, Duration::from_millis(500));
        let attached = window.attachedSheet();
        let sheet_window = host.ns_window(sheet.id());
        c.check(
            attached.as_ref().zip(sheet_window.as_ref()).is_some_and(|(a, s)| std::ptr::eq::<NSWindow>(&**a, &***s)),
            "a sheet window is attached to its parent",
        );
        host.update(|cx| cx.close_window(sheet.id()));
        pump(&host, Duration::from_millis(500));
        c.check(window.attachedSheet().is_none(), "closing the sheet ends it on the parent");

        // Settings window.
        let settings = host.update(|cx| open_settings(cx, |_, cx| cx.new(|_| Sheet)));
        pump(&host, Duration::from_millis(400));
        let settings_window = settings.and_then(|s| host.ns_window(s.id()));
        c.check(
            settings_window.as_ref().is_some_and(|w| w.toolbarStyle() == NSWindowToolbarStyle::Preference),
            "open_settings() opens a preference-style window",
        );
        let again = host.update(|cx| open_settings(cx, |_, cx| cx.new(|_| Sheet)));
        c.check(again.is_none(), "a second open_settings() brings the open one forward");

        // Embedded traffic lights.
        let row = host.update(|cx| {
            cx.open_window(
                WindowOptions::main("chrome.row")
                    .title("Row")
                    .titlebar(Titlebar::Custom)
                    .size(size(px(420.), px(200.))),
                |_, cx| cx.new(|_| Row),
            )
        });
        pump(&host, Duration::from_millis(600));
        let row_window = host.ns_window(row.id()).expect("row window");
        let close = row_window.standardWindowButton(NSWindowButton::CloseButton).expect("close button");
        let close_x = close.convertRect_toView(close.bounds(), None).origin.x;
        c.check(
            (close_x - 60.).abs() < 0.5,
            &format!("embedded traffic lights sit in their slot (close x = {close_x}, slot x = 60)"),
        );
        shot(&row_window, "chrome-embedded");

        let _ = std::fs::remove_dir_all(&dir);
        if c.0 == 0 {
            println!("ND_CHROME_OK");
            std::process::exit(0);
        }
        println!("ND_CHROME_FAILED {} check(s)", c.0);
        std::process::exit(1);
    }
}
