//! Tier 2: the command palette, source tree, toasts and banner on the AppKit backend and real
//! windows. Every check reads AppKit's own state back (the field editor's text and selection,
//! the outline's rows and item objects, the toast's view), and user input goes through AppKit's
//! own paths: the field editor's `insertText:` and `doCommandBySelector:`, the outline's
//! `expandItem:` and selection, the buttons' actions. Reentrancy: every handler sets the
//! widget's props from inside its own event. Prints `ND_NAV_COMPONENTS_OK`.

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main("gate_m6::appkit_nav_components", || nav::run());
    #[cfg(not(target_os = "macos"))]
    harness::main("gate_m6::appkit_nav_components", || {});
}

#[cfg(target_os = "macos")]
mod nav {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use nd_backend_appkit::objc2::rc::Retained;
    use nd_backend_appkit::objc2::runtime::AnyObject;
    use nd_backend_appkit::objc2::sel;
    use nd_backend_appkit::objc2_app_kit::{NSResponder, NSTableViewStyle, NSTextView, NSView};
    use nd_backend_appkit::objc2_foundation::{MainThreadMarker, NSIndexSet, NSNotFound, NSRange, NSString};
    use nd_backend_appkit::{AppKitBackend, NdAppKitPlatform, WindowHost};
    use nd_components::*;
    use nd_core::semantic::Role;
    use nd_ui::prelude::*;
    use nd_ui::{App, NodeId, Size, WindowId};

    #[derive(Default)]
    struct Log {
        queries: Vec<String>,
        activated: Vec<String>,
        cancels: u32,
        selected: Vec<String>,
        expanded: Vec<String>,
        tree_actions: Vec<(String, String)>,
        dismissed: Vec<(String, DismissReason)>,
        toast_actions: Vec<String>,
        banner_clicks: u32,
    }

    const NOTES: [&str; 3] = ["Trip to Lisbon", "Groceries", "Trivia night"];

    struct Nav {
        log: Rc<RefCell<Log>>,
        palette_mounted: bool,
        open: bool,
        query: SharedString,
        nodes: Vec<SourceNode>,
        selected: Option<SharedString>,
        toasts: Vec<Toast>,
        banner: bool,
    }

    impl Nav {
        fn items(&self) -> Vec<PaletteItem> {
            let q = self.query.to_lowercase();
            NOTES
                .iter()
                .filter(|t| t.to_lowercase().contains(&q))
                .map(|t| {
                    let item = palette_item(*t, *t).icon(Icon::FileText).subtitle("Notes").hint("Open");
                    if !q.is_empty() && t.to_lowercase().starts_with(&q) { item.completion(*t) } else { item }
                })
                .collect()
        }
    }

    impl Render for Nav {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let palette = self.palette_mounted.then(|| {
                command_palette("palette")
                    .open(self.open)
                    .placeholder("Search notes")
                    .query(self.query.clone())
                    .items(self.items())
                    .on_query_change(cx.listener(|this, e: &PaletteQueryChanged, _, cx| {
                        this.log.borrow_mut().queries.push(e.text.to_string());
                        this.query = e.text.clone();
                        cx.notify();
                    }))
                    .on_activate(cx.listener(|this, e: &PaletteActivated, _, cx| {
                        this.log.borrow_mut().activated.push(e.id.to_string());
                        this.open = false;
                        cx.notify();
                    }))
                    .on_cancel(cx.listener(|this, _: &PaletteCancelled, _, cx| {
                        this.log.borrow_mut().cancels += 1;
                        this.open = false;
                        cx.notify();
                    }))
            });
            h_flex()
                .size_full()
                .child(
                    div().w(px(240.)).h_full().child(
                        source_tree("sidebar")
                            .nodes(self.nodes.clone())
                            .actions([source_action("close", Icon::Trash).tooltip("Close")])
                            .selected_opt(self.selected.clone())
                            .empty_state(Some(Icon::Folder), "No folders", Some("Add one with the plus button".into()))
                            .on_select(cx.listener(|this, e: &SourceSelected, _, cx| {
                                this.log.borrow_mut().selected.push(e.id.to_string());
                                this.selected = Some(e.id.clone());
                                cx.notify();
                            }))
                            .on_expand(cx.listener(|this, e: &SourceExpanded, _, cx| {
                                this.log.borrow_mut().expanded.push(e.id.to_string());
                                if let Some(n) = this.nodes.iter_mut().find(|n| n.id == e.id) {
                                    n.expanded = true;
                                }
                                cx.notify();
                            }))
                            .on_action(cx.listener(|this, e: &SourceActionClicked, _, cx| {
                                this.log.borrow_mut().tree_actions.push((e.id.to_string(), e.action.to_string()));
                                cx.notify();
                            })),
                    ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .h_full()
                        .child(
                            banner("banner", "An update is ready").action("Restart").revealed(self.banner).on_action(
                                cx.listener(|this, _: &BannerActionClicked, _, cx| {
                                    this.log.borrow_mut().banner_clicks += 1;
                                    this.banner = false;
                                    cx.notify();
                                }),
                            ),
                        )
                        .child(div().flex_1())
                        .children(palette)
                        .child(
                            toast_overlay("toasts")
                                .toasts(self.toasts.clone())
                                .on_dismiss(cx.listener(|this, e: &ToastDismissed, _, cx| {
                                    this.log.borrow_mut().dismissed.push((e.id.to_string(), e.reason));
                                    this.toasts.retain(|t| t.id != e.id);
                                    cx.notify();
                                }))
                                .on_action(cx.listener(|this, e: &ToastActionClicked, _, cx| {
                                    this.log.borrow_mut().toast_actions.push(e.id.to_string());
                                    cx.notify();
                                })),
                        ),
                )
        }
    }

    fn nodes() -> Vec<SourceNode> {
        vec![
            source_section("lib", "Library"),
            source_node("inbox", "Inbox").parent("lib").icon(Icon::Inbox).badge("3"),
            source_node("work", "Work").parent("lib").icon(Icon::Folder).actions(["close"]),
            source_node("q3", "Q3").parent("work").caption("Planning"),
        ]
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

    struct Env {
        app: App,
        platform: NdAppKitPlatform,
        backend: AppKitBackend,
    }

    impl Env {
        /// Native callbacks wake the host on a later turn; this is that turn, then the frame.
        fn settle(&mut self) {
            for _ in 0..4 {
                self.platform.pump(Duration::from_millis(30), Duration::from_millis(300));
                self.app.drain_events();
                self.app.frame();
            }
        }

        fn node(&self, test_id: &str) -> NodeId {
            let tree = self.app.semantic_tree();
            tree.nodes().find(|(_, n)| n.test_id.as_deref() == Some(test_id)).map(|(_, n)| n.id).expect(test_id)
        }
    }

    /// The palette draws in the overlay window over the handle's window (6.3); its field
    /// editor is that window's first responder.
    fn field_editor(view: &NSView) -> Option<Retained<NSTextView>> {
        let window = view.window()?;
        let windows = window.childWindows().map(|c| c.to_vec()).unwrap_or_default();
        windows.iter().chain(std::iter::once(&window)).find_map(|w| {
            let responder: Retained<NSResponder> = w.firstResponder()?;
            let any: Retained<AnyObject> = responder.into();
            any.downcast::<NSTextView>().ok()
        })
    }

    pub fn run() -> ! {
        let mtm = MainThreadMarker::new().expect("main thread");
        let platform = NdAppKitPlatform::new(mtm);
        let backend = AppKitBackend::new(mtm);
        backend.on_wake(|| {});
        let mut app = App::with_backend(Box::new(backend.clone()));
        let log: Rc<RefCell<Log>> = Rc::default();
        let l = log.clone();
        let handle = app.open_window(WindowOptions::main("nav"), move |_, cx| {
            cx.new(|_| Nav {
                log: l,
                palette_mounted: true,
                open: false,
                query: "gro".into(),
                nodes: nodes(),
                selected: None,
                toasts: Vec::new(),
                banner: true,
            })
        });
        let id: WindowId = handle.id();
        let window = platform.open_window("Nav", 900., 640.);
        let _host = WindowHost::new(&backend, id, &window);
        app.resize_window(id, Size { width: 900., height: 640. });
        let mut env = Env { app, platform, backend };
        env.settle();
        let mut c = Checks(0);

        palette(&mut env, &handle, &log, &mut c);
        source_tree_checks(&mut env, &handle, &log, &mut c);
        toasts(&mut env, &handle, &log, &mut c);
        banner_checks(&mut env, &handle, &log, &mut c);

        window.close();
        if c.0 == 0 {
            println!("ND_NAV_COMPONENTS_OK");
            std::process::exit(0);
        }
        println!("ND_NAV_COMPONENTS_FAILED {} check(s)", c.0);
        std::process::exit(1);
    }

    fn palette(env: &mut Env, handle: &WindowHandle<Nav>, log: &Rc<RefCell<Log>>, c: &mut Checks) {
        let node = env.node("palette");
        let closed = env.app.semantic_tree();
        let semantic = closed.find(node).map(|(_, n)| n.clone()).expect("palette node");
        c.check(
            semantic.role == Role::Dialog && semantic.name == "Search notes" && !semantic.visible,
            &format!(
                "closed, the palette is an invisible dialog named by its placeholder ({:?} {:?})",
                semantic.role, semantic.name
            ),
        );

        handle.root().update(&mut env.app, |this, cx| {
            this.open = true;
            cx.notify();
        });
        env.settle();
        let Some(layout) = env.backend.palette_layout(node) else {
            c.check(false, "opening presents the palette");
            return;
        };
        let height = layout.window.size.height;
        c.check(
            (layout.card.size.width - 640.).abs() < 0.5
                && (layout.card.origin.y - (height * 0.18).round()).abs() <= 1.
                && (layout.card.origin.x - (900. - 640.) / 2.).abs() <= 1.,
            &format!(
                "the card is 640 wide, centred, its top at 18% of the window ({:?} in {:?})",
                layout.card, layout.window
            ),
        );
        c.check(
            layout.material.contains("NSGlassEffectView") || layout.material.contains("NSVisualEffectView"),
            &format!("the card is the system material ({})", layout.material),
        );
        c.check(
            layout.field_focused
                && layout.field_text == "gro"
                && layout.selection.location == 0
                && layout.selection.length == 3,
            &format!(
                "the field has focus and starts from the app's query, all selected ({:?} {:?} focused={})",
                layout.field_text, layout.selection, layout.field_focused
            ),
        );
        c.check(
            layout.rows.len() == 1 && layout.rows[0].highlighted && layout.rows[0].icon.is_some(),
            &format!("one row for the seeded query, highlighted, its icon shown ({} rows)", layout.rows.len()),
        );
        if let Some(row) = layout.rows.first() {
            let hint = row.hint.expect("hint");
            let sub = row.subtitle.expect("subtitle");
            c.check(
                (row.row.origin.x + row.row.size.width - (hint.origin.x + hint.size.width) - 10.).abs() <= 1.
                    && sub.origin.x > row.title.origin.x + row.title.size.width
                    && row.row.size.height <= 40.5,
                &format!("one-line row: subtitle after the title, hint at the trailing edge ({row:?})"),
            );
        }
        let semantic = env.app.semantic_tree().find(node).map(|(_, n)| n.clone()).expect("palette");
        c.check(
            semantic.expanded == Some(true)
                && semantic.value.as_deref() == Some("gro")
                && semantic.bounds.is_some_and(|b| b.size.width == 640.),
            &format!(
                "open, the dialog is expanded, holds the field's text and has the card's bounds ({:?})",
                semantic.value
            ),
        );

        // Type over the selected seed through the field editor, one keystroke at a time.
        let Some(view) = env.backend.view(node) else { return };
        let editor = field_editor(&view);
        c.check(editor.is_some(), "the field editor is the window's first responder");
        let Some(editor) = editor else { return };
        // The window turning key as the palette opens (as on coming back to the app) leaves
        // the keyboard with the palette.
        let panel_key = view
            .window()
            .and_then(|w| w.childWindows())
            .is_some_and(|c| c.to_vec().iter().any(|w| w.canBecomeKeyWindow() && w.isKeyWindow()));
        c.check(panel_key, "the open palette's panel is the key window");
        for key in ["t", "r", "i"] {
            let typed = NSString::from_str(key);
            let at = NSRange::new(NSNotFound as usize, 0);
            let _: () =
                unsafe { nd_backend_appkit::objc2::msg_send![&*editor, insertText: &*typed, replacementRange: at] };
            env.settle();
        }
        let layout = env.backend.palette_layout(node).expect("still open");
        c.check(
            log.borrow().queries == ["t", "tr", "tri"],
            &format!("each edit reaches the app once, without the completion ({:?})", log.borrow().queries),
        );
        c.check(
            layout.field_text == "trip to Lisbon" && layout.selection.location == 3 && layout.selection.length == 11,
            &format!(
                "the first row's completion shows selected after the caret, the app's echo reset nothing ({:?} {:?})",
                layout.field_text, layout.selection
            ),
        );
        let _: () = unsafe { nd_backend_appkit::objc2::msg_send![&*editor, doCommandBySelector: sel!(insertTab:)] };
        env.settle();
        let layout = env.backend.palette_layout(node).expect("still open");
        c.check(
            log.borrow().queries.last().map(String::as_str) == Some("trip to Lisbon")
                && layout.selection.location == 14
                && layout.selection.length == 0,
            &format!(
                "Tab accepts the completion: the app sees the whole text, caret at the end ({:?})",
                layout.selection
            ),
        );
        let _: () = unsafe { nd_backend_appkit::objc2::msg_send![&*editor, doCommandBySelector: sel!(insertNewline:)] };
        env.settle();
        c.check(
            log.borrow().activated == ["Trip to Lisbon"]
                && env.backend.palette_layout(node).is_none()
                && log.borrow().cancels == 0,
            &format!(
                "Return activates the row; the app's close tears it down with no cancel ({:?})",
                log.borrow().activated
            ),
        );

        handle.root().update(&mut env.app, |this, cx| {
            this.open = true;
            this.query = "".into();
            cx.notify();
        });
        env.settle();
        let rows = env.backend.palette_layout(node).map(|l| l.rows.len()).unwrap_or(0);
        if let Some(editor) = field_editor(&view) {
            let _: () = unsafe { nd_backend_appkit::objc2::msg_send![&*editor, doCommandBySelector: sel!(moveDown:)] };
            env.settle();
            let highlighted = env.backend.palette_layout(node).and_then(|l| l.rows.iter().position(|r| r.highlighted));
            c.check(
                rows == 3 && highlighted == Some(1),
                &format!("Down moves the highlight ({highlighted:?} of {rows})"),
            );
            let _: () =
                unsafe { nd_backend_appkit::objc2::msg_send![&*editor, doCommandBySelector: sel!(cancelOperation:)] };
            env.settle();
        }
        c.check(
            log.borrow().cancels == 1 && env.backend.palette_layout(node).is_none(),
            &format!("Esc cancels once and closes ({} cancels)", log.borrow().cancels),
        );

        // Unmounted while open: the backdrop in the overlay window goes with it.
        handle.root().update(&mut env.app, |this, cx| {
            this.open = true;
            cx.notify();
        });
        env.settle();
        let owner = view.window();
        let backdrops = || {
            let layers = owner.as_ref().and_then(|w| w.childWindows()).map(|c| c.to_vec()).unwrap_or_default();
            layers
                .iter()
                .filter_map(|w| w.contentView())
                .flat_map(|h| h.subviews().to_vec())
                .filter(|v| v.class().name().to_str().is_ok_and(|n| n.contains("PaletteBackdrop")))
                .count()
        };
        let before = backdrops();
        handle.root().update(&mut env.app, |this, cx| {
            this.palette_mounted = false;
            cx.notify();
        });
        env.settle();
        c.check(
            before == 1 && backdrops() == 0 && log.borrow().cancels == 1,
            &format!("an unmount while open removes the backdrop silently ({before} -> {})", backdrops()),
        );
    }

    fn source_tree_checks(env: &mut Env, handle: &WindowHandle<Nav>, log: &Rc<RefCell<Log>>, c: &mut Checks) {
        let node = env.node("sidebar");
        let Some(outline) = env.backend.source_outline(node) else {
            c.check(false, "the source tree is an NSOutlineView");
            return;
        };
        c.check(outline.style() == NSTableViewStyle::SourceList, "the source tree is a source-list NSOutlineView");
        c.check(
            outline.numberOfRows() == 3,
            &format!("section, two rows, Work collapsed ({} rows)", outline.numberOfRows()),
        );
        let inbox = env.backend.source_item(node, "inbox").expect("inbox item");

        // The user selects Inbox; the app feeds it back from its handler.
        let row = unsafe { outline.rowForItem(Some(&inbox)) };
        outline.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(row as usize), false);
        env.settle();
        c.check(
            log.borrow().selected == ["inbox"],
            &format!("a user selection reaches the app once, fed back without an echo ({:?})", log.borrow().selected),
        );

        // The user opens Work's disclosure; the app records the expansion and re-renders.
        let work = env.backend.source_item(node, "work").expect("work item");
        let work_any: &AnyObject = &work;
        unsafe { outline.expandItem(Some(work_any)) };
        env.settle();
        c.check(
            log.borrow().expanded == ["work"]
                && outline.numberOfRows() == 4
                && unsafe { outline.isItemExpanded(Some(work_any)) },
            &format!("a disclosure click reaches the app once and stays open ({:?})", log.borrow().expanded),
        );

        // A nodes update keeps every row's object, so selection and expansion survive.
        handle.root().update(&mut env.app, |this, cx| {
            this.nodes[1].title = "Inbox (renamed)".into();
            cx.notify();
        });
        env.settle();
        let same = env.backend.source_item(node, "inbox").is_some_and(|i| std::ptr::eq(&*i, &*inbox));
        let selected =
            outline.itemAtRow(outline.selectedRow()).and_then(|i| i.downcast::<NSString>().ok()).map(|s| s.to_string());
        c.check(
            same && selected.as_deref() == Some("inbox") && unsafe { outline.isItemExpanded(Some(work_any)) },
            &format!("a nodes update reuses items by id: selection and open branches stay ({same}, {selected:?})"),
        );
        c.check(log.borrow().selected.len() == 1, "the reload reported no selection");

        handle.root().update(&mut env.app, |this, cx| {
            this.selected = Some("q3".into());
            cx.notify();
        });
        env.settle();
        let selected =
            outline.itemAtRow(outline.selectedRow()).and_then(|i| i.downcast::<NSString>().ok()).map(|s| s.to_string());
        c.check(
            selected.as_deref() == Some("q3") && log.borrow().selected.len() == 1,
            &format!("the app's selection reaches the outline without a report ({selected:?})"),
        );

        let hidden = env.backend.source_actions(node, "work", None);
        let shown = env.backend.source_actions(node, "work", Some(true));
        c.check(
            hidden == [0.] && shown == [1.],
            &format!("trailing actions show on hover only ({hidden:?} -> {shown:?})"),
        );
        let clicked = env.backend.source_click_action(node, "work", "close");
        env.settle();
        c.check(
            clicked && log.borrow().tree_actions == [("work".to_string(), "close".to_string())],
            &format!("an action click reports the row and the action ({:?})", log.borrow().tree_actions),
        );

        let tree = env.app.semantic_tree();
        let sidebar = tree.find(node).map(|(_, n)| n.clone()).expect("sidebar");
        c.check(
            sidebar.role == Role::Tree && sidebar.value.as_deref() == Some("q3"),
            &format!(
                "the semantic tree has a tree with its selection as value ({:?} {:?})",
                sidebar.role, sidebar.value
            ),
        );

        handle.root().update(&mut env.app, |this, cx| {
            this.nodes.clear();
            cx.notify();
        });
        env.settle();
        c.check(
            env.backend.source_empty_title(node).as_deref() == Some("No folders"),
            "with no rows the empty state shows",
        );
        handle.root().update(&mut env.app, |this, cx| {
            this.nodes = nodes();
            cx.notify();
        });
        env.settle();
    }

    fn toasts(env: &mut Env, handle: &WindowHandle<Nav>, log: &Rc<RefCell<Log>>, c: &mut Checks) {
        let node = env.node("toasts");
        handle.root().update(&mut env.app, |this, cx| {
            this.toasts = vec![
                toast("saved", "Saved").timeout(Duration::from_millis(300)),
                toast("deleted", "Note deleted").action("Undo").persistent(),
            ];
            cx.notify();
        });
        env.settle();
        let shown = env.backend.toast_shown(node);
        c.check(
            shown.as_ref().is_some_and(|(t, b, ..)| t == "Saved" && b.is_none()),
            &format!("the first toast shows alone ({shown:?})"),
        );
        if let Some((_, _, frame, material)) = &shown {
            let window = env.backend.view(node).and_then(|v| v.window()).expect("window");
            let host = window.contentView().and_then(|v| unsafe { v.superview() }).expect("frame view");
            let size = host.bounds().size;
            c.check(
                ((frame.origin.x + frame.size.width / 2.) - size.width / 2.).abs() <= 1.
                    && (size.height - (frame.origin.y + frame.size.height) - 28.).abs() <= 1.
                    && (material.contains("Glass") || material.contains("VisualEffect")),
                &format!("bottom-centre on the system material ({frame:?} in {size:?}, {material})"),
            );
        }
        // Its 300 ms run out.
        for _ in 0..8 {
            env.platform.pump(Duration::from_millis(100), Duration::from_millis(150));
        }
        env.settle();
        let shown = env.backend.toast_shown(node);
        c.check(
            log.borrow().dismissed == [("saved".to_string(), DismissReason::Timeout)]
                && shown.as_ref().is_some_and(|(t, b, ..)| t == "Note deleted" && b.as_deref() == Some("Undo")),
            &format!("a timeout dismisses and the queue moves on ({:?}, {shown:?})", log.borrow().dismissed),
        );
        env.backend.toast_click(node, true);
        env.settle();
        c.check(
            log.borrow().toast_actions == ["deleted"]
                && log.borrow().dismissed.last() == Some(&("deleted".to_string(), DismissReason::Action)),
            &format!("the button reports and dismisses ({:?})", log.borrow().toast_actions),
        );

        handle.root().update(&mut env.app, |this, cx| {
            this.toasts = vec![toast("a", "Syncing").persistent()];
            cx.notify();
        });
        env.settle();
        handle.root().update(&mut env.app, |this, cx| {
            this.toasts.push(toast("b", "You are offline").high_priority().persistent());
            cx.notify();
        });
        env.settle();
        let shown = env.backend.toast_shown(node);
        c.check(
            log.borrow().dismissed.last() == Some(&("a".to_string(), DismissReason::Interrupted))
                && shown.as_ref().is_some_and(|(t, ..)| t == "You are offline"),
            &format!("a high-priority toast interrupts ({shown:?})"),
        );
        let tree = env.app.semantic_tree();
        let overlay = tree.find(node).map(|(_, n)| n.clone()).expect("overlay");
        c.check(
            overlay.role == Role::Status
                && overlay.name == "You are offline"
                && overlay.bounds.is_some_and(|b| b.size.width > 0.),
            &format!(
                "the toast is a status with the toast's bounds in the semantic tree ({:?} {:?})",
                overlay.role, overlay.name
            ),
        );
        handle.root().update(&mut env.app, |this, cx| {
            this.toasts.clear();
            cx.notify();
        });
        env.settle();
        c.check(env.backend.toast_shown(node).is_none(), "a toast the app removes leaves");
    }

    fn banner_checks(env: &mut Env, handle: &WindowHandle<Nav>, log: &Rc<RefCell<Log>>, c: &mut Checks) {
        let node = env.node("banner");
        let view = env.backend.view(node).expect("banner view");
        let height = view.frame().size.height;
        c.check(!view.isHidden() && height > 0., &format!("a revealed banner takes its height ({height})"));
        env.app.perform(node, &nd_ui::semantics::SemanticAction::Click).ok();
        env.settle();
        c.check(
            log.borrow().banner_clicks == 1 && view.isHidden(),
            &format!("its button reaches the app, which hides it from the handler ({})", log.borrow().banner_clicks),
        );
        let _ = handle;
    }
}
