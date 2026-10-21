//! The overlay leak gate on GPUI, in-process (ARCHITECTURE.md 6.3): a popover, a modal panel and
//! a context menu over a native button and a GPU surface. Presses, moves and scrolls inside an
//! overlay never reach what lies under it; a press outside a light-dismiss overlay closes it and
//! is consumed; Esc closes the innermost overlay. Every event goes through the GPUI window's own
//! input path, as the platform would deliver it.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use nd_backend_gpui::{GpuiBackend, GpuiChrome, GpuiHost};
use nd_components::{Chrome, Menu, MenuItem, SubtreeOverrides};
use nd_gpui::gpui::{
    self, HeadlessAppContext, Modifiers, MouseButton as GpuiButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PlatformInput, ScrollDelta, ScrollWheelEvent, TouchPhase, point,
};
use nd_ui::backend::Backend;
use nd_ui::prelude::*;
use nd_ui::{InputCounts, KeyBinding, Keystroke, SemanticNode, SurfaceSource, WindowId};

gpui::actions!(test, [Chosen]);

#[derive(Default)]
struct Scene {
    popover: bool,
    panel: bool,
    cover: bool,
    cover_light: bool,
    cover_field: bool,
    cover_taps: u32,
    under: u32,
    chosen: u32,
    taps: u32,
    log: Rc<RefCell<Vec<String>>>,
}

impl Render for Scene {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("scene")
            .size_full()
            .p(px(10.))
            .on_action(cx.listener(|this, _: &Chosen, _, cx| {
                this.chosen += 1;
                cx.notify();
            }))
            .child(div().id("anchor").w(px(300.)).h(px(30.)).when(self.popover, |d| {
                d.child(popover("pop").w(px(280.)).h(px(160.)).child(text("inside")).on_dismiss(cx.listener(
                    |this, e: &Dismissed, _, cx| {
                        this.log.borrow_mut().push(format!("popover {:?}", e.reason));
                        this.popover = false;
                        cx.notify();
                    },
                )))
            }))
            .child(
                h_flex()
                    .items_start()
                    .gap(px(10.))
                    .child(button("under", "Under").w(px(140.)).h(px(220.)).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.under += 1;
                            cx.notify();
                        },
                    )))
                    .child(
                        div()
                            .id("menu-zone")
                            .on_tap(cx.listener(|this, _: &TapEvent, _, cx| {
                                this.taps += 1;
                                cx.notify();
                            }))
                            .context_menu(|menu, _, _| {
                                menu.item("Choose", Chosen)
                                    .separator()
                                    .item("Again", Chosen)
                                    .submenu(Menu::new("More").item(MenuItem::action("Deeper", Chosen)))
                            })
                            .child(
                                native_surface("surface", SurfaceSource::TestPattern)
                                    .count_input()
                                    .w(px(140.))
                                    .h(px(220.)),
                            ),
                    ),
            )
            .child(text(format!("under={} chosen={}", self.under, self.chosen)).id("status"))
            .when(self.cover, |d| {
                let light = self.cover_light;
                let field = self.cover_field;
                d.child(
                    cover("cover")
                        .when(light, |c| c.modality(Modality::LightDismiss))
                        .on_dismiss(cx.listener(|this, e: &Dismissed, _, cx| {
                            this.log.borrow_mut().push(format!("cover {:?}", e.reason));
                            this.cover = false;
                            cx.notify();
                        }))
                        .child(
                            v_flex()
                                .size_full()
                                .p(px(40.))
                                .child(div().id("cover-zone").size_full().on_tap(cx.listener(
                                    |this, _: &TapEvent, _, cx| {
                                        this.cover_taps += 1;
                                        cx.notify();
                                    },
                                )))
                                .when(field, |v| v.child(nd_components::forms::text_field("cover-field").w(px(100.)))),
                        ),
                )
            })
            .when(self.panel, |d| {
                d.child(panel("panel").w(px(200.)).h(px(80.)).child(text("panel")).on_dismiss(cx.listener(
                    |this, e: &Dismissed, _, cx| {
                        this.log.borrow_mut().push(format!("panel {:?}", e.reason));
                        this.panel = false;
                        cx.notify();
                    },
                )))
            })
    }
}

struct Opened {
    cx: HeadlessAppContext,
    host: GpuiHost,
    window: WindowId,
    scene: Entity<Scene>,
}

fn headless() -> HeadlessAppContext {
    let text = nd_gpui::gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text, Arc::new(()), nd_gpui::gpui_platform::current_headless_renderer)
}

fn settle(cx: &mut HeadlessAppContext, host: &GpuiHost) {
    for _ in 0..4 {
        cx.run_until_parked();
        cx.update(|cx| {
            host.pump(cx);
            host.draw(cx);
        });
    }
    cx.run_until_parked();
}

fn open() -> Opened {
    let mut cx = headless();
    let mut opened = None;
    let host = cx.update(|cx| {
        GpuiHost::builder().quit_when_empty(false).start(cx, |app| {
            let backend = app.backend_as::<GpuiBackend>().expect("GPUI backend").clone();
            app.set_global(Chrome(GpuiChrome::new(backend)));
            app.bind_keys([KeyBinding::new("ctrl-k", Chosen, None)]);
            let handle = app.open_window(WindowOptions::main("main").size(size(px(480.), px(360.))), |_, cx| {
                cx.new(|_| Scene::default())
            });
            opened = Some((handle.id(), handle.root().clone()));
        })
    });
    settle(&mut cx, &host);
    let (window, scene) = opened.expect("window");
    Opened { cx, host, window, scene }
}

fn find<'a>(node: &'a SemanticNode, test_id: &str) -> Option<&'a SemanticNode> {
    if node.test_id.as_deref() == Some(test_id) {
        return Some(node);
    }
    node.children.iter().find_map(|c| find(c, test_id))
}

fn by_test_id(host: &GpuiHost, window: WindowId, test_id: &str) -> Option<SemanticNode> {
    host.read(|app| find(&app.semantic_window(window).expect("window").root, test_id).cloned())
}

fn center(host: &GpuiHost, window: WindowId, test_id: &str) -> nd_ui::Point {
    let b = by_test_id(host, window, test_id).and_then(|n| n.bounds).unwrap_or_else(|| panic!("{test_id} bounds"));
    nd_ui::Point { x: b.origin.x + b.size.width / 2., y: b.origin.y + b.size.height / 2. }
}

fn counts(host: &GpuiHost, window: WindowId) -> InputCounts {
    let value = by_test_id(host, window, "surface").and_then(|n| n.value).expect("surface counts");
    InputCounts::parse(&value).expect("counts")
}

fn taps(o: &Opened) -> u32 {
    o.host.read(|app| o.scene.read(app).taps)
}

fn under(o: &Opened) -> u32 {
    o.host.read(|app| o.scene.read(app).under)
}

/// Posts platform events to the window, as the OS would deliver them, and settles.
fn post(o: &mut Opened, events: Vec<PlatformInput>) {
    let handle = o.host.window_handle(o.window).expect("handle");
    for event in events {
        o.cx.update(|cx| {
            let _ = handle.update(cx, |_, window, cx| {
                window.dispatch_event(event, cx);
            });
        });
        settle(&mut o.cx, &o.host);
    }
}

fn click(at: nd_ui::Point, button: GpuiButton) -> Vec<PlatformInput> {
    let position = point(px(at.x), px(at.y));
    let modifiers = Modifiers::default();
    vec![
        PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers }),
        PlatformInput::MouseDown(MouseDownEvent { button, position, modifiers, click_count: 1, first_mouse: false }),
        PlatformInput::MouseUp(MouseUpEvent { button, position, modifiers, click_count: 1 }),
    ]
}

fn wiggle_and_scroll(at: nd_ui::Point) -> Vec<PlatformInput> {
    let modifiers = Modifiers::default();
    let mut events: Vec<PlatformInput> = (0..4)
        .map(|i| {
            let position = point(px(at.x + i as f32 * 3.), px(at.y));
            PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers })
        })
        .collect();
    events.push(PlatformInput::ScrollWheel(ScrollWheelEvent {
        position: point(px(at.x), px(at.y)),
        delta: ScrollDelta::Pixels(point(px(0.), px(-40.))),
        modifiers,
        touch_phase: TouchPhase::Moved,
    }));
    events
}

fn wiggle(at: nd_ui::Point) -> Vec<PlatformInput> {
    let modifiers = Modifiers::default();
    (0..3)
        .map(|i| {
            let position = point(px(at.x + i as f32), px(at.y));
            PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers })
        })
        .collect()
}

fn set(o: &mut Opened, f: impl FnOnce(&mut Scene)) {
    let scene = o.scene.clone();
    o.cx.update(|cx| {
        o.host.update(cx, |app| {
            scene.update(app, |s, cx| {
                f(s);
                cx.notify();
            })
        })
    });
    settle(&mut o.cx, &o.host);
}

fn log(o: &Opened) -> Vec<String> {
    o.host.read(|app| o.scene.read(app).log.borrow().clone())
}

mod gate_m4_overlays {
    use super::*;

    pub fn gpui_popover_captures_input_over_button_and_surface() {
        let mut o = open();
        // The surface and button are live: one probe each before the overlay opens.
        let surface = center(&o.host, o.window, "surface");
        let button = center(&o.host, o.window, "under");
        set(&mut o, |s| s.popover = true);
        let pop = by_test_id(&o.host, o.window, "pop").and_then(|n| n.bounds).expect("popover bounds");
        let inside = |p: nd_ui::Point| {
            p.x > pop.origin.x
                && p.x < pop.origin.x + pop.size.width
                && p.y > pop.origin.y
                && p.y < pop.origin.y + pop.size.height
        };
        // Points inside the popover that lie over the button and over the surface.
        let over_button = nd_ui::Point { x: button.x, y: pop.origin.y + pop.size.height / 2. };
        let over_surface = nd_ui::Point { x: surface.x.min(pop.origin.x + pop.size.width - 10.), y: over_button.y };
        assert!(inside(over_button) && inside(over_surface), "the popover covers both: {pop:?}");

        for at in [over_button, over_surface] {
            post(&mut o, click(at, GpuiButton::Left));
            post(&mut o, click(at, GpuiButton::Right));
            post(&mut o, wiggle_and_scroll(at));
        }
        assert_eq!(under(&o), 0, "the button under the popover was clicked");
        assert_eq!(taps(&o), 0, "a tap recognizer under the popover fired");
        assert_eq!(counts(&o.host, o.window), InputCounts::default(), "input reached the surface under the popover");
        assert!(log(&o).is_empty(), "a press inside closed the popover: {:?}", log(&o));

        // Outside, on the surface below the popover: closes it, consumed.
        let outside = nd_ui::Point { x: surface.x, y: pop.origin.y + pop.size.height + 20. };
        assert!(!inside(outside));
        post(&mut o, click(outside, GpuiButton::Left));
        assert_eq!(log(&o), ["popover OutsidePress"]);
        assert!(by_test_id(&o.host, o.window, "pop").is_none(), "the popover closed");
        let after = counts(&o.host, o.window);
        assert_eq!((after.down, after.up), (0, 0), "the dismissing press passed through: {after}");

        assert_eq!(taps(&o), 0, "the dismissing press reached a tap recognizer");

        // With it gone, the same press reaches the surface, its tap recognizer and the button.
        post(&mut o, click(outside, GpuiButton::Left));
        assert_eq!(counts(&o.host, o.window).down, 1);
        assert_eq!(taps(&o), 1);
        post(&mut o, click(button, GpuiButton::Left));
        assert_eq!(under(&o), 1);
    }

    /// A closed popover keeps drawing its content for the look's exit, takes no input while it
    /// does, and is gone once the exit is over.
    pub fn gpui_a_closed_popover_animates_out_then_is_gone() {
        let mut o = open();
        let button = center(&o.host, o.window, "under");
        set(&mut o, |s| s.popover = true);
        let inside =
            |o: &Opened| o.host.backend().paints(o.window).expect("paints").iter().any(|p| p.label == "inside");
        assert!(inside(&o), "the open popover is drawn");
        set(&mut o, |s| s.popover = false);
        assert!(inside(&o), "right after closing, the popover is still drawn, leaving");
        post(&mut o, click(button, GpuiButton::Left));
        assert_eq!(o.host.read(|app| o.scene.read(app).under), 1, "the leaving popover takes no input");
        std::thread::sleep(std::time::Duration::from_millis(300));
        settle(&mut o.cx, &o.host);
        assert!(!inside(&o), "after its exit the popover is gone");
    }

    pub fn gpui_modal_panel_blocks_outside_and_escape_closes() {
        let mut o = open();
        let button = center(&o.host, o.window, "under");
        let surface = center(&o.host, o.window, "surface");
        set(&mut o, |s| s.panel = true);
        assert!(by_test_id(&o.host, o.window, "panel").and_then(|n| n.bounds).is_some(), "panel laid out");
        post(&mut o, click(button, GpuiButton::Left));
        post(&mut o, click(surface, GpuiButton::Left));
        post(&mut o, wiggle_and_scroll(surface));
        assert_eq!(under(&o), 0);
        let c = counts(&o.host, o.window);
        assert_eq!((c.down, c.up, c.moved, c.wheel), (0, 0, 0, 0), "a modal panel lets nothing outside it through");
        assert!(log(&o).is_empty(), "outside presses never close a modal panel");
        let escape = Keystroke::parse("escape").expect("escape");
        o.cx.update(|cx| {
            o.host.update(cx, |app| app.synthesize(o.window, &nd_ui::semantics::SyntheticInput::Key(escape)))
        })
        .expect("posted");
        settle(&mut o.cx, &o.host);
        assert_eq!(log(&o), ["panel Escape"]);
        post(&mut o, click(button, GpuiButton::Left));
        assert_eq!(under(&o), 1, "the button works once the panel is gone");
    }

    pub fn gpui_cover_fills_the_window_and_taps_reach_its_content() {
        let mut o = open();
        let button = center(&o.host, o.window, "under");
        set(&mut o, |s| s.cover = true);
        let bounds = by_test_id(&o.host, o.window, "cover").and_then(|n| n.bounds).expect("cover bounds");
        assert_eq!((bounds.size.width, bounds.size.height), (480., 360.), "the cover fills the window");
        post(&mut o, click(button, GpuiButton::Left));
        assert_eq!(under(&o), 0, "nothing under the cover takes a press");
        assert_eq!(o.host.read(|app| o.scene.read(app).cover_taps), 1, "a tap inside the cover reaches its content");
    }

    /// A light-dismiss cover closes on a press on its scrim, the padding around its content
    /// here, and the press goes no further; a press on what it holds reaches that.
    pub fn gpui_a_light_dismiss_cover_closes_on_its_scrim_only() {
        let mut o = open();
        let button = center(&o.host, o.window, "under");
        set(&mut o, |s| {
            s.cover = true;
            s.cover_light = true;
        });
        post(&mut o, click(button, GpuiButton::Left));
        assert_eq!(o.host.read(|app| o.scene.read(app).cover_taps), 1, "a tap on its content reaches it");
        assert!(log(&o).is_empty(), "a press on the content leaves the cover open");
        post(&mut o, click(nd_ui::Point { x: 20., y: 20. }, GpuiButton::Left));
        assert_eq!(log(&o), ["cover OutsidePress"], "a press on the scrim closes it");
        assert!(by_test_id(&o.host, o.window, "cover").is_none(), "the cover is gone");
        assert_eq!(under(&o), 0, "the press closed the cover and went no further");
        post(&mut o, click(button, GpuiButton::Left));
        assert_eq!(under(&o), 1, "the button works once the cover is gone");
    }

    pub fn gpui_context_menu_is_drawn_above_and_consumes() {
        let mut o = open();
        let surface = center(&o.host, o.window, "surface");
        let button = center(&o.host, o.window, "under");
        let open_at = nd_ui::Point { x: surface.x - 50., y: surface.y - 60. };
        post(&mut o, click(open_at, GpuiButton::Right));
        assert!(o.host.backend().menu_open(o.window), "a right click on the zone opens its menu");
        let before = counts(&o.host, o.window);
        // The first item sits just below the press, over the surface.
        let item = nd_ui::Point { x: open_at.x + 30., y: open_at.y + 14. };
        post(&mut o, wiggle_and_scroll(item));
        post(&mut o, click(item, GpuiButton::Left));
        assert!(!o.host.backend().menu_open(o.window), "choosing an item closes the menu");
        assert_eq!(o.host.read(|app| o.scene.read(app).chosen), 1, "the item dispatched its action");
        assert_eq!(counts(&o.host, o.window), before, "input on the menu reached the surface under it");

        post(&mut o, click(open_at, GpuiButton::Right));
        assert!(o.host.backend().menu_open(o.window));
        post(&mut o, click(button, GpuiButton::Left));
        assert!(!o.host.backend().menu_open(o.window), "a press outside closes the menu");
        assert_eq!(under(&o), 0, "and is consumed");

        post(&mut o, click(open_at, GpuiButton::Right));
        let escape = Keystroke::parse("escape").expect("escape");
        o.cx.update(|cx| {
            o.host.update(cx, |app| app.synthesize(o.window, &nd_ui::semantics::SyntheticInput::Key(escape)))
        })
        .expect("posted");
        settle(&mut o.cx, &o.host);
        assert!(!o.host.backend().menu_open(o.window), "Esc closes the menu");
    }

    /// A submenu opens beside its row on hover, and its items dispatch like the menu's own.
    pub fn gpui_context_submenu_opens_on_hover() {
        let mut o = open();
        let surface = center(&o.host, o.window, "surface");
        let open_at = nd_ui::Point { x: surface.x - 50., y: surface.y - 90. };
        post(&mut o, click(open_at, GpuiButton::Right));
        assert!(o.host.backend().menu_open(o.window));
        let drawn = |o: &Opened, label: &str| {
            o.host.backend().paints(o.window).expect("paints").iter().any(|p| p.label == label)
        };
        assert!(drawn(&o, "More"), "the submenu's row is drawn");
        assert!(!drawn(&o, "Deeper"), "the submenu stays shut until its row is hovered");
        // Choose, a separator, Again, then More: 28 px rows, a 9 px separator, under the menu's
        // 4 px inset and 1 px border.
        let more = nd_ui::Point { x: open_at.x + 30., y: open_at.y + 5. + 28. + 9. + 28. + 14. };
        post(&mut o, wiggle(more));
        assert!(drawn(&o, "Deeper"), "hovering More opens its submenu");
        // The submenu's first row is level with More, past the menu's 180 px.
        let deeper = nd_ui::Point { x: open_at.x + 220., y: more.y };
        post(&mut o, wiggle(deeper));
        assert!(drawn(&o, "Deeper"), "the submenu stays open under the pointer");
        post(&mut o, click(deeper, GpuiButton::Left));
        assert!(!o.host.backend().menu_open(o.window), "choosing a submenu item closes the menu");
        assert_eq!(o.host.read(|app| o.scene.read(app).chosen), 1, "the submenu item dispatched its action");

        post(&mut o, click(open_at, GpuiButton::Right));
        post(&mut o, wiggle(more));
        assert!(drawn(&o, "Deeper"));
        post(&mut o, wiggle(nd_ui::Point { x: open_at.x + 30., y: open_at.y + 5. + 14. }));
        assert!(!drawn(&o, "Deeper"), "hovering another row shuts the submenu");
    }

    /// A closed cover still draws its controls while it animates out, but keeps no state for
    /// them once their nodes are released: a control drawn then must not start anything that
    /// outlives the exit (a video would play on unheard).
    pub fn gpui_a_closed_cover_keeps_no_control_state() {
        let mut o = open();
        set(&mut o, |s| {
            s.cover = true;
            s.cover_field = true;
        });
        let field = by_test_id(&o.host, o.window, "cover-field").expect("the field is drawn").id;
        assert!(o.host.backend().holds_control_state(field), "the open cover's field keeps its state");
        set(&mut o, |s| s.cover = false);
        settle(&mut o.cx, &o.host);
        assert!(!o.host.backend().holds_control_state(field), "the leaving cover's field keeps none");
        std::thread::sleep(std::time::Duration::from_millis(300));
        settle(&mut o.cx, &o.host);
        assert!(!o.host.backend().holds_control_state(field), "nor after its exit");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tests: Vec<(&str, fn())> = vec![
        (
            "gate_m4_overlays::gpui_popover_captures_input_over_button_and_surface",
            gate_m4_overlays::gpui_popover_captures_input_over_button_and_surface,
        ),
        (
            "gate_m4_overlays::gpui_a_closed_popover_animates_out_then_is_gone",
            gate_m4_overlays::gpui_a_closed_popover_animates_out_then_is_gone,
        ),
        (
            "gate_m4_overlays::gpui_modal_panel_blocks_outside_and_escape_closes",
            gate_m4_overlays::gpui_modal_panel_blocks_outside_and_escape_closes,
        ),
        (
            "gate_m4_overlays::gpui_cover_fills_the_window_and_taps_reach_its_content",
            gate_m4_overlays::gpui_cover_fills_the_window_and_taps_reach_its_content,
        ),
        (
            "gate_m4_overlays::gpui_a_light_dismiss_cover_closes_on_its_scrim_only",
            gate_m4_overlays::gpui_a_light_dismiss_cover_closes_on_its_scrim_only,
        ),
        (
            "gate_m4_overlays::gpui_context_menu_is_drawn_above_and_consumes",
            gate_m4_overlays::gpui_context_menu_is_drawn_above_and_consumes,
        ),
        (
            "gate_m4_overlays::gpui_context_submenu_opens_on_hover",
            gate_m4_overlays::gpui_context_submenu_opens_on_hover,
        ),
        (
            "gate_m4_overlays::gpui_a_closed_cover_keeps_no_control_state",
            gate_m4_overlays::gpui_a_closed_cover_keeps_no_control_state,
        ),
    ];
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--list") {
        if !has("--ignored") {
            for (name, _) in &tests {
                println!("{name}: test");
            }
        }
        return;
    }
    if has("--ignored") {
        return;
    }
    let exact = has("--exact");
    let mut skip = false;
    let filters: Vec<&String> = args
        .iter()
        .filter(|a| {
            if std::mem::take(&mut skip) {
                return false;
            }
            if matches!(a.as_str(), "--format" | "--test-threads" | "--color" | "--skip" | "-Z") {
                skip = true;
            }
            !a.starts_with('-')
        })
        .collect();
    let mut failed = 0;
    for (name, test) in &tests {
        let selected =
            filters.is_empty() || filters.iter().any(|f| if exact { *f == *name } else { name.contains(f.as_str()) });
        if !selected {
            continue;
        }
        match std::panic::catch_unwind(*test) {
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        std::process::exit(101);
    }
}
