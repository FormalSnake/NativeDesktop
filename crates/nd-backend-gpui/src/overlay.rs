//! The overlay layer on GPUI (ARCHITECTURE.md 6.3): every overlay draws in GPUI's deferred layer,
//! above all content of its window and above every native surface element, nested overlays above
//! their parents. Input is captured there:
//! - The overlay's content occludes what is under it, so presses, scrolls and hover never reach
//!   a button or surface below.
//! - A non-passive overlay lays a transparent backdrop over the window. A press on it is
//!   consumed; on a light-dismiss overlay it also asks the overlay to close. A modal backdrop
//!   blocks scrolling outside too; a light-dismiss one lets scrolling through to content outside
//!   the overlay, as AppKit does.
//! - A cover fills the window, so its outside is its scrim: a press where nothing it holds draws
//!   or takes input closes a light-dismiss cover. Scrolling never passes a cover.
//! - Passive overlays (tooltips) take no input.
//!
//! Context menus are a drawn menu in the same layer, opened through [`GpuiChrome`]; items
//! dispatch their action from the window's focused node, as a key binding would.
//!
//! Native surfaces composited by the OS above GPUI's base layer (WebView2, WPE subsurfaces) need
//! gpui-fast's overlay composition surface to stay under this layer; that arrives with the M8
//! engines. Today's GPU test surface draws inside GPUI, so the deferred layer covers it already.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use nd_components::services::{AlertCallback, ChromeServices, OpenCallback, SaveCallback};
use nd_components::{
    AboutPanel, Alert, ContextMenu, ImageFit, ImagePipeline, ImageProps, Menu, MenuItem, OpenDialog, SaveDialog,
};
use nd_core::{NodeId, Point, Rect, WindowId};
use nd_gpui::gpui::{
    self, Anchor, AnyElement, Bounds, DispatchPhase, ElementId, HitboxBehavior, InteractiveElement, IntoElement,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Refineable, ScrollWheelEvent, SharedString,
    StatefulInteractiveElement, Styled, anchored, canvas, deferred, div, fill, hsla, point, px,
};
use nd_theme::color::with_alpha;
use nd_ui::backend::NodeKind;
use nd_ui::{DismissCause, Dismissed, DivProps, Modality, OverlayKind, OverlayProps, Placement, SurfaceProps};

use crate::backend::{GpuiBackend, Node};
use crate::occlusion::Occluding;
use crate::render::{Build, Tracked};

/// Stacking inside GPUI's deferred layer, one order for every overlay kind: toasts lowest (they
/// never block), framework panels (the command palette) and overlays above them, nested overlays
/// above their parents, the context menu on top.
/// Under toasts and panels: a shared element in flight only needs to clear the content.
pub(crate) const HERO_PRIORITY: usize = 0;
pub(crate) const TOAST_PRIORITY: usize = 1;
pub(crate) const PANEL_PRIORITY: usize = 10;
const MENU_PRIORITY: usize = usize::MAX;

/// The gap between an anchor and what it anchors.
const GAP: f32 = 4.;
/// A panel's top edge sits this far down the window, as a fraction of its height.
const PANEL_TOP: f32 = 0.18;

/// An open drawn context menu: where, and its items.
pub(crate) struct MenuOverlay {
    /// The node that asked for it, which its paints are recorded under.
    pub node: NodeId,
    pub at: Point,
    /// `at` is the menu's bottom left corner rather than its top left.
    pub above: bool,
    pub content: MenuContent,
    /// The open submenus, each one's index among the items of the level above.
    pub open: Vec<usize>,
}

pub(crate) enum MenuContent {
    /// The app's menu: a chosen item dispatches its action from the focused node.
    Actions(Vec<MenuItem>),
    /// A presenter's menu ([`GpuiBackend::show_menu`]): the choice goes to its callback.
    Rows(Vec<MenuRow>, Option<MenuResponse>),
}

/// One row of a menu a presenter shows with [`GpuiBackend::show_menu`].
#[derive(Clone, Debug, PartialEq)]
pub enum MenuRow {
    Item { label: SharedString, enabled: bool, checked: bool },
    Separator,
}

/// Told the index of the row picked in a menu shown with [`GpuiBackend::show_menu`], or `None`
/// when it closed without a pick.
pub type MenuResponse = Box<dyn FnOnce(Option<usize>)>;

/// Whether a press at `at` falls on the scrim of the cover `cover`: nothing it holds draws or
/// takes input there. A container counts through a fill or a gesture of its own, and a picture
/// fitted inside its box only where it is drawn.
fn on_scrim(
    nodes: &std::collections::HashMap<NodeId, Node>,
    bounds: &std::collections::HashMap<NodeId, Rect>,
    cover: NodeId,
    at: Point,
) -> bool {
    fn contains(r: &Rect, p: Point) -> bool {
        p.x >= r.origin.x && p.y >= r.origin.y && p.x < r.origin.x + r.size.width && p.y < r.origin.y + r.size.height
    }
    fn shows(
        nodes: &std::collections::HashMap<NodeId, Node>,
        bounds: &std::collections::HashMap<NodeId, Rect>,
        id: NodeId,
        at: Point,
    ) -> bool {
        let Some(node) = nodes.get(&id) else { return false };
        let div = node.props.downcast_ref::<DivProps>();
        if div.is_some_and(|d| d.style.visibility == Some(gpui::Visibility::Hidden))
            || node.props.downcast_ref::<OverlayProps>().is_some()
        {
            return false;
        }
        let drawn = bounds.get(&id).is_some_and(|b| match (node.kind, div) {
            (NodeKind::View, Some(d)) => {
                let fill = d
                    .style
                    .background
                    .as_ref()
                    .and_then(|f| f.color())
                    .and_then(|b| b.as_solid())
                    .is_some_and(|c| c.a > 0.);
                (fill || !d.gestures.is_empty()) && contains(b, at)
            }
            (NodeKind::View, None) => false,
            _ => match node.props.downcast_ref::<ImageProps>() {
                Some(p) if p.fit == ImageFit::Contain => {
                    let (w, h) = p.intrinsic(ImagePipeline::global()).unwrap_or((0, 0));
                    if w == 0 || h == 0 {
                        contains(b, at)
                    } else {
                        let scale = (b.size.width / w as f32).min(b.size.height / h as f32);
                        let size = nd_core::Size { width: w as f32 * scale, height: h as f32 * scale };
                        let origin = Point {
                            x: b.origin.x + (b.size.width - size.width) / 2.,
                            y: b.origin.y + (b.size.height - size.height) / 2.,
                        };
                        contains(&Rect { origin, size }, at)
                    }
                }
                _ => contains(b, at),
            },
        });
        drawn || node.children.iter().any(|c| shows(nodes, bounds, *c, at))
    }
    nodes.get(&cover).is_some_and(|n| !n.children.iter().any(|c| shows(nodes, bounds, *c, at)))
}

impl Build<'_> {
    pub(crate) fn overlay_depth(&self, id: NodeId) -> usize {
        let mut depth = 0;
        let mut at = self.nodes.get(&id).and_then(|n| n.parent);
        while let Some(n) = at {
            // A leaving overlay's snapshot holds only its own subtree.
            let Some(node) = self.nodes.get(&n) else { break };
            if node.props.downcast_ref::<OverlayProps>().is_some() {
                depth += 1;
            }
            at = node.parent;
        }
        depth
    }

    pub(crate) fn overlay(&self, id: NodeId, node: &Node, window: &mut gpui::Window, cx: &mut gpui::App) -> AnyElement {
        self.overlay_with(id, node, None, window, cx)
    }

    /// The overlay `id`, or with `exit` its ghost animating out: `exit` is the eased progress
    /// and the anchor's bounds as they were. A ghost takes no input.
    pub(crate) fn overlay_with(
        &self,
        id: NodeId,
        node: &Node,
        exit: Option<(f32, Option<Rect>)>,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> AnyElement {
        let props = node.props.downcast_ref::<OverlayProps>().expect("Overlay node needs OverlayProps");
        let theme = self.theme();
        let passive = props.modality == Modality::Passive || exit.is_some();
        // GTK 4's popovers close at once, so libadwaita's look leaves no ghost to animate out.
        let closes_animated = theme.preset != nd_theme::Preset::Adwaita;
        if exit.is_none() && props.modality != Modality::Passive && closes_animated {
            self.backend.overlay_exits.seen(
                id,
                self.window_id,
                self.subtree(id),
                node.parent.and_then(|p| self.backend.bounds(p)),
            );
        }
        let mut content = div().id(ElementId::Integer(id.0));
        content.style().refine(&props.style);
        content.style().position = Some(gpui::Position::Relative);
        // The app's own background, or the theme's overlay surface; content builds over it.
        let styled = props.style.background.as_ref().and_then(|f| f.color()).and_then(|b| b.as_solid());
        let cover = props.kind == OverlayKind::Cover;
        let adwaita = theme.preset == nd_theme::Preset::Adwaita;
        // libadwaita's tooltip: 80% near-black under white text, whatever the window's scheme.
        let adw_tip = adwaita && props.kind == OverlayKind::Tooltip;
        let fill = styled.unwrap_or(if cover {
            theme.scrim
        } else if adw_tip {
            with_alpha(theme.scrim, 0.8)
        } else {
            theme.surface.overlay
        });
        let ink = if adw_tip { theme.text.on_osd } else { theme.text.primary };
        if cover {
            content = content.bg(fill).size_full();
        } else if styled.is_none() && adw_tip {
            content = content
                .bg(fill)
                .text_color(ink)
                .border_1()
                .border_color(with_alpha(gpui::white(), 0.1))
                .rounded(theme.radius.md)
                .px(px(10.))
                .py(px(6.));
        } else if styled.is_none() && adwaita {
            // `popover > contents`: 15 px corners, a 14% black edge and two soft shadows.
            let look = crate::forms::FormLook::of(&theme);
            content = content
                .bg(fill)
                .border_1()
                .border_color(look.popover_stroke)
                .rounded(theme.radius.xl)
                .shadow(look.popover_shadows());
        } else if styled.is_none() {
            // WinUI's ToolTip takes the control radius, its flyouts the overlay radius.
            let fluent_tip = props.kind == OverlayKind::Tooltip && theme.preset == nd_theme::Preset::Fluent;
            let radius = if fluent_tip { theme.radius.md } else { theme.radius.lg };
            content = content.bg(fill).border_1().rounded(radius);
            content = if theme.preset == nd_theme::Preset::Fluent {
                // SurfaceStrokeColorFlyout and the flyout shadow, as the drawn menus.
                let look = crate::forms::look::FormLook::fluent(&theme);
                content.border_color(look.popover_stroke).shadow(vec![gpui::BoxShadow {
                    color: look.popover_shadow.0,
                    offset: point(px(0.), look.popover_shadow.1),
                    blur_radius: look.popover_shadow.2,
                    spread_radius: px(0.),
                    inset: false,
                }])
            } else {
                content.border_color(theme.border.subtle).shadow_md()
            };
        }
        let children: Vec<AnyElement> = self.painted(id, Some(fill), Some(ink), "overlay", || {
            node.children.iter().map(|c| self.node(*c, window, cx)).collect()
        });
        if !passive {
            let scrim = (cover && props.modality == Modality::LightDismiss).then(|| {
                let emit = self.backend.state.borrow().events.emitter(id);
                (emit, self.wake.clone(), self.backend.clone(), self.window_id)
            });
            content = self
                .feed_gestures(content)
                .occlude()
                .on_any_mouse_down(move |event, _, cx| {
                    cx.stop_propagation();
                    let Some((emit, wake, backend, window_id)) = &scrim else { return };
                    let at = Point { x: event.position.x.as_f32(), y: event.position.y.as_f32() };
                    let on_scrim = {
                        let (state, bounds) = (backend.state.borrow(), backend.bounds.borrow());
                        on_scrim(&state.nodes, &bounds, id, at)
                    };
                    if event.button == gpui::MouseButton::Left && on_scrim {
                        backend.swallow_release(*window_id);
                        emit.emit(Dismissed { reason: DismissCause::OutsidePress });
                        wake(cx);
                    }
                })
                // The root's wheel handler cannot see through the occluding surface either, so a
                // scroll view inside the overlay is fed from here.
                .on_scroll_wheel({
                    let (backend, wake, window_id) = (self.backend.clone(), self.wake.clone(), self.window_id);
                    move |event, window, cx| {
                        cx.stop_propagation();
                        backend.gesture_input(window_id, window, cx, &wake, |g, tree| g.scroll(window_id, event, tree));
                    }
                });
        }
        // Popovers, menus and panels come in with the look's entrance: a short rise and a fade
        // over its normal duration (WinUI's "fast out, slow in", 167 ms; the panel takes the slow
        // one), or at once when the OS has animations off. Tooltips just appear.
        let content = content.children(children);
        let reduce = self.backend.platform_reduce_motion() == Some(true);
        // A cover is the whole window: it fades, it does not rise.
        let rise = match props.placement {
            _ if cover => 0.,
            Placement::Above => -8.,
            _ => 8.,
        };
        let content = if let Some((t, _)) = exit {
            content.opacity(1. - t).mt(px(rise * t)).into_any_element()
        } else if passive || reduce || adwaita {
            // GTK 4's popovers open at once.
            content.into_any_element()
        } else {
            let fluent = theme.preset == nd_theme::Preset::Fluent;
            let duration = if props.kind == OverlayKind::Panel { theme.motion.slow } else { theme.motion.normal };
            let ease: fn(f32) -> f32 = if fluent {
                |t| crate::forms::look::cubic_bezier([0., 0., 0., 1.], t)
            } else {
                |t| 1. - (1. - t).powi(3)
            };
            gpui::AnimationExt::with_animation(
                content,
                ElementId::NamedInteger("nd-overlay-enter".into(), id.0),
                gpui::Animation::new(duration).with_easing(ease),
                move |el, t| el.opacity(t).mt(px(rise * (1. - t))),
            )
            .into_any_element()
        };
        let content = Tracked::new(id, content, self.backend.bounds.clone());
        let content = if passive { Occluding::passive(content) } else { Occluding::surface(content) };

        let viewport = window.viewport_size();
        let positioned = if cover {
            div().absolute().top_0().left_0().w(viewport.width).h(viewport.height).child(content).into_any_element()
        } else if props.kind == OverlayKind::Panel {
            div()
                .absolute()
                .top_0()
                .left_0()
                .w(viewport.width)
                .h(viewport.height)
                .flex()
                .flex_col()
                .items_center()
                .pt(viewport.height * PANEL_TOP)
                .child(content)
                .into_any_element()
        } else {
            let mut layer = anchored().snap_to_window_with_margin(px(8.));
            let anchor = match exit {
                Some((_, anchor)) => anchor,
                None => node.parent.and_then(|p| self.backend.bounds(p)),
            };
            if let Some(a) = anchor {
                let (left, top, right, bottom) =
                    (a.origin.x, a.origin.y, a.origin.x + a.size.width, a.origin.y + a.size.height);
                let (at, corner) = match props.placement {
                    Placement::Below => (point(px(left), px(bottom + GAP)), Anchor::TopLeft),
                    Placement::Above => (point(px(left), px(top - GAP)), Anchor::BottomLeft),
                    Placement::Leading => (point(px(left - GAP), px(top)), Anchor::TopRight),
                    Placement::Trailing => (point(px(right + GAP), px(top)), Anchor::TopLeft),
                };
                layer = layer.position(at).anchor(corner);
            }
            layer.child(content).into_any_element()
        };

        let layer = if passive {
            positioned
        } else {
            let emit = self.backend.state.borrow().events.emitter(id);
            let (wake, backend, window_id) = (self.wake.clone(), self.backend.clone(), self.window_id);
            let light = props.modality == Modality::LightDismiss;
            let mut backdrop = self.feed_gestures(div().id(ElementId::Name(format!("nd-backdrop-{}", id.0).into())));
            backdrop = if light && !cover { backdrop.block_mouse_except_scroll() } else { backdrop.occlude() };
            let backdrop = backdrop
                .absolute()
                .top_0()
                .left_0()
                .w(viewport.width)
                .h(viewport.height)
                .on_any_mouse_down(move |_, _, cx| {
                    cx.stop_propagation();
                    backend.swallow_release(window_id);
                    if light {
                        emit.emit(Dismissed { reason: DismissCause::OutsidePress });
                        wake(cx);
                    }
                })
                .child(positioned);
            anchored().position(point(px(0.), px(0.))).child(Occluding::backdrop(backdrop)).into_any_element()
        };
        deferred(layer).with_priority(PANEL_PRIORITY + 1 + self.overlay_depth(id)).into_any_element()
    }

    /// Hands the pointer to nd's gesture recognizers from inside the overlay layer. The host
    /// feeds them from the window's root, whose hitbox an occluding overlay hides, so a tap inside
    /// an overlay would otherwise never be seen. Where the press lands is still nd's hit test.
    fn feed_gestures(&self, element: gpui::Stateful<gpui::Div>) -> gpui::Stateful<gpui::Div> {
        let feed = |phase: nd_gesture::PointerPhase| {
            let (backend, wake, window_id) = (self.backend.clone(), self.wake.clone(), self.window_id);
            move |at: gpui::Point<Pixels>, window: &mut gpui::Window, cx: &mut gpui::App| {
                backend.gesture_pointer(
                    window_id,
                    phase,
                    Point { x: at.x.as_f32(), y: at.y.as_f32() },
                    &wake,
                    window,
                    cx,
                )
            }
        };
        let (down, up, moved) = (
            feed(nd_gesture::PointerPhase::Down),
            feed(nd_gesture::PointerPhase::Up),
            feed(nd_gesture::PointerPhase::Move),
        );
        element
            .capture_any_mouse_down(move |e, window, cx| {
                if e.button == gpui::MouseButton::Left {
                    down(e.position, window, cx);
                }
            })
            .capture_any_mouse_up(move |e, window, cx| {
                if e.button == gpui::MouseButton::Left {
                    up(e.position, window, cx);
                }
            })
            .on_mouse_move(move |e, window, cx| {
                if e.pressed_button == Some(gpui::MouseButton::Left) {
                    moved(e.position, window, cx);
                }
            })
    }

    /// A native surface. The GPU test pattern draws stripes and counts the pointer input that
    /// reaches it: an event counts only when the surface's hitbox is the one under the pointer,
    /// the same test GPUI's own handlers apply. Web pages need the M8 engines; until then they show
    /// the pattern too.
    pub(crate) fn surface(&self, id: NodeId, node: &Node) -> AnyElement {
        let props = node.props.downcast_ref::<SurfaceProps>().expect("NativeSurface node needs SurfaceProps");
        let counts = self.backend.surface_counts.clone();
        let wake = self.wake.clone();
        let mut element = canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |bounds: Bounds<Pixels>, hitbox, window, _| {
                window.paint_quad(fill(bounds, hsla(0.6, 0.6, 0.45, 1.)));
                let stripe = bounds.size.width / 8.;
                for i in (0..8).step_by(2) {
                    let origin = point(bounds.origin.x + stripe * i as f32, bounds.origin.y);
                    window.paint_quad(fill(
                        Bounds::new(origin, gpui::size(stripe, bounds.size.height)),
                        hsla(0.6, 0.7, 0.55, 1.),
                    ));
                }
                let count = |f: fn(&mut nd_ui::InputCounts)| {
                    let counts = counts.clone();
                    let wake = wake.clone();
                    move |window: &gpui::Window, cx: &mut gpui::App| {
                        f(counts.borrow_mut().entry(id).or_default());
                        let _ = window;
                        wake(cx);
                    }
                };
                let (h, down) = (hitbox.clone(), count(|c| c.down += 1));
                window.on_mouse_event(move |_: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble && h.is_hovered(window) {
                        down(window, cx);
                    }
                });
                let (h, up) = (hitbox.clone(), count(|c| c.up += 1));
                window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble && h.is_hovered(window) {
                        up(window, cx);
                    }
                });
                let (h, moved) = (hitbox.clone(), count(|c| c.moved += 1));
                window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble && h.is_hovered(window) {
                        moved(window, cx);
                    }
                });
                let (h, wheel) = (hitbox, count(|c| c.wheel += 1));
                window.on_mouse_event(move |_: &ScrollWheelEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble && h.should_handle_scroll(window) {
                        wheel(window, cx);
                    }
                });
            },
        );
        element.style().refine(&props.style);
        element.into_any_element()
    }

    /// The window's open context menu, drawn above everything, with a light-dismiss backdrop.
    pub(crate) fn menu(&self, menu: &MenuOverlay, window: &gpui::Window) -> AnyElement {
        let theme = self.theme();
        let marks = crate::nav::Marks::default();
        let (surface, body) = (theme.surface.overlay, theme.type_scale.body);
        let regular = gpui::FontWeight::NORMAL;
        marks.fill("context menu", surface, &[]);
        let viewport = window.viewport_size();
        let window_id = self.window_id;
        // WinUI's MenuFlyout: the overlay radius, SurfaceStrokeColorFlyout and the flyout shadow,
        // 32 px items inset 4 across and 2 along with padding 11, subtle fills, a 28 px check
        // column (MenuFlyout_themeresources.xaml).
        let fluent = (theme.preset == nd_theme::Preset::Fluent).then(|| crate::forms::look::FormLook::fluent(&theme));
        let list = |id: &'static str| {
            let list =
                div().id(id).flex().flex_col().min_w(px(180.)).bg(surface).border_1().text_color(theme.text.primary);
            let list = match fluent {
                Some(look) => list.py(px(2.)).rounded(theme.radius.lg).border_color(look.popover_stroke).shadow(vec![
                    gpui::BoxShadow {
                        color: look.popover_shadow.0,
                        offset: point(px(0.), look.popover_shadow.1),
                        blur_radius: look.popover_shadow.2,
                        spread_radius: px(0.),
                        inset: false,
                    },
                ]),
                None => list.py(px(4.)).rounded(theme.radius.md).border_color(theme.border.subtle).shadow_md(),
            };
            list.occlude().on_any_mouse_down(|_, _, cx| cx.stop_propagation())
        };
        let entry = || match fluent {
            Some(look) => {
                div().mx(px(4.)).my(px(2.)).px(look.padding_x).min_h(px(32.)).flex().items_center().rounded(look.radius)
            }
            None => div().px(px(12.)).py(px(4.)),
        };
        let hovered = match fluent {
            Some(look) => look.subtle_hover,
            None => theme.hover(surface),
        };
        let row = |i: usize, marked: bool, label: &SharedString, enabled: bool, checked: bool| {
            let mut row = entry().id(ElementId::Integer(i as u64));
            if marked {
                let column = if fluent.is_some() { px(28.) } else { px(16.) };
                row = row.flex().child(div().w(column).child(if checked { "\u{2713}" } else { "" }));
            }
            let row = row.child(label.clone());
            if enabled {
                marks.text(label, theme.text.primary, &[surface], body, regular);
                let pressed = match fluent {
                    Some(look) => look.subtle_active,
                    None => theme.hover(surface),
                };
                row.hover(move |s| s.bg(hovered)).active(move |s| s.bg(pressed))
            } else {
                marks.text(label, theme.text.tertiary, &[surface], body, regular);
                row.text_color(theme.text.tertiary)
            }
        };
        let separator = || {
            marks.fill("menu separator", theme.separator, &[surface]);
            let line = div().h(px(1.)).bg(theme.separator);
            match fluent {
                // MenuFlyoutSeparator: full width, 1 px clear of the items' own 2 px margins.
                Some(_) => line.my(px(1.)).into_any_element(),
                None => line.my(px(4.)).into_any_element(),
            }
        };
        let list = match &menu.content {
            MenuContent::Actions(items) => {
                // Each level of submenus, the open one's index in the level above it.
                let mut levels: Vec<(&[MenuItem], Option<usize>)> = vec![(items, None)];
                for &at in &menu.open {
                    let (level, _) = levels[levels.len() - 1];
                    let Some(MenuItem::Submenu(sub)) = level.get(at) else { break };
                    let last = levels.len() - 1;
                    levels[last].1 = Some(at);
                    levels.push((&sub.items, None));
                }
                // Built from the innermost out: a submenu hangs off its row in the level above.
                let mut child: Option<AnyElement> = None;
                for (depth, (items, open)) in levels.iter().enumerate().rev() {
                    let marked = items.iter().any(|i| matches!(i, MenuItem::Action { checked: true, .. }));
                    let mut level = list(if depth == 0 { "nd-context-menu" } else { "nd-context-submenu" });
                    for (i, item) in items.iter().enumerate() {
                        // Hovering a row of this level opens its submenu, or closes the open one.
                        let (backend, wake) = (self.backend.clone(), self.wake.clone());
                        let path: Vec<usize> = menu.open.iter().copied().take(depth).collect();
                        let hover = move |sub: Option<usize>| {
                            let (backend, wake, path) = (backend.clone(), wake.clone(), path.clone());
                            move |hovered: &bool, _: &mut gpui::Window, cx: &mut gpui::App| {
                                if *hovered {
                                    let mut open = path.clone();
                                    open.extend(sub);
                                    if backend.open_submenu(window_id, open) {
                                        wake(cx);
                                    }
                                }
                            }
                        };
                        level = level.child(match item {
                            MenuItem::Separator => separator(),
                            MenuItem::Action { label, action, enabled, checked, .. } => {
                                let row = row(i, marked, label, *enabled, *checked).on_hover(hover(None));
                                if *enabled {
                                    let (backend, wake, action) =
                                        (self.backend.clone(), self.wake.clone(), action.boxed_clone());
                                    row.on_click(move |_, _, cx| {
                                        backend.choose(window_id, action.boxed_clone());
                                        wake(cx);
                                    })
                                    .into_any_element()
                                } else {
                                    row.into_any_element()
                                }
                            }
                            MenuItem::Submenu(sub) => {
                                let shown = *open == Some(i);
                                let mut row = row(i, marked, &sub.title, true, false)
                                    .relative()
                                    .flex()
                                    .justify_between()
                                    .gap(px(16.))
                                    .child("\u{203a}")
                                    .on_hover(hover(Some(i)));
                                if shown {
                                    row = row.bg(hovered);
                                    if let Some(sub) = child.take() {
                                        // From the row to the menu's outer edge: the row's inset
                                        // and the border.
                                        let edge = if fluent.is_some() { px(5.) } else { px(1.) };
                                        row = row
                                            .child(Beside { edge, child: Occluding::surface(sub).into_any_element() });
                                    }
                                }
                                row.into_any_element()
                            }
                            MenuItem::Role(_) => div().into_any_element(),
                        });
                    }
                    child = Some(level.into_any_element());
                }
                child.expect("a menu has a top level")
            }
            MenuContent::Rows(rows, _) => {
                let marked = rows.iter().any(|r| matches!(r, MenuRow::Item { checked: true, .. }));
                let mut level = list("nd-context-menu");
                for (i, item) in rows.iter().enumerate() {
                    level = level.child(match item {
                        MenuRow::Separator => separator(),
                        MenuRow::Item { label, enabled, checked } => {
                            let row = row(i, marked, label, *enabled, *checked);
                            if *enabled {
                                let (backend, wake) = (self.backend.clone(), self.wake.clone());
                                row.on_click(move |_, _, cx| {
                                    backend.respond(window_id, Some(i));
                                    wake(cx);
                                })
                                .into_any_element()
                            } else {
                                row.into_any_element()
                            }
                        }
                    });
                }
                level.into_any_element()
            }
        };
        self.record_marks(menu.node, marks.take());
        let backend = self.backend.clone();
        let wake = self.wake.clone();
        let backdrop = div()
            .id("nd-context-menu-backdrop")
            .block_mouse_except_scroll()
            .absolute()
            .top_0()
            .left_0()
            .w(viewport.width)
            .h(viewport.height)
            .on_any_mouse_down(move |_, _, cx| {
                cx.stop_propagation();
                backend.swallow_release(window_id);
                backend.close_menu(window_id);
                wake(cx);
            })
            .child(
                anchored()
                    .position(point(px(menu.at.x), px(menu.at.y)))
                    .anchor(if menu.above { Anchor::BottomLeft } else { Anchor::TopLeft })
                    .snap_to_window_with_margin(px(8.))
                    .child(Occluding::surface(list)),
            );
        deferred(anchored().position(point(px(0.), px(0.))).child(Occluding::backdrop(backdrop)))
            .with_priority(MENU_PRIORITY)
            .into_any_element()
    }
}

impl MenuOverlay {
    /// The menu closed without a pick: a presenter's callback hears `None`.
    fn dismissed(self) {
        if let MenuContent::Rows(_, Some(respond)) = self.content {
            respond(None);
        }
    }
}

impl GpuiBackend {
    /// Closes the window's context menu and queues `action` for its focused node.
    pub(crate) fn choose(&self, window: WindowId, action: Box<dyn gpui::Action>) {
        let mut state = self.state.borrow_mut();
        state.menus.remove(&window);
        state.dirty.insert(window);
        state.actions.push((window, action));
    }

    pub(crate) fn close_menu(&self, window: WindowId) -> bool {
        let closed = {
            let mut state = self.state.borrow_mut();
            let closed = state.menus.remove(&window);
            if closed.is_some() {
                state.dirty.insert(window);
            }
            closed
        };
        match closed {
            Some(menu) => {
                menu.dismissed();
                true
            }
            None => false,
        }
    }

    /// Closes the window's presenter menu and tells its callback what was picked.
    fn respond(&self, window: WindowId, row: Option<usize>) {
        let menu = {
            let mut state = self.state.borrow_mut();
            state.dirty.insert(window);
            state.menus.remove(&window)
        };
        if let Some(MenuOverlay { content: MenuContent::Rows(_, Some(respond)), .. }) = menu {
            respond(row);
        }
    }

    /// Pops a menu of `rows` up over `node` at `at` (the node's coordinates) in `window`, drawn
    /// like the app's context menus, and hands `respond` the picked row, or `None` once it closes
    /// without a pick (Esc, a press outside, another menu). For presenters with menus of their
    /// own: a web engine's context menu.
    pub fn show_menu(&self, window: WindowId, node: NodeId, at: Point, rows: Vec<MenuRow>, respond: MenuResponse) {
        let origin = self.bounds(node).map_or(Point::default(), |b| b.origin);
        let content = MenuContent::Rows(rows, Some(respond));
        let replaced = {
            let mut state = self.state.borrow_mut();
            state.dirty.insert(window);
            state.menus.insert(
                window,
                MenuOverlay {
                    node,
                    at: Point { x: origin.x + at.x, y: origin.y + at.y },
                    above: false,
                    content,
                    open: Vec::new(),
                },
            )
        };
        if let Some(old) = replaced {
            old.dismissed();
        }
        self.wake();
    }

    /// Shows the submenus at `open` in the window's context menu. False when they already show.
    pub(crate) fn open_submenu(&self, window: WindowId, open: Vec<usize>) -> bool {
        let mut state = self.state.borrow_mut();
        let Some(menu) = state.menus.get_mut(&window) else { return false };
        if menu.open == open {
            return false;
        }
        menu.open = open;
        state.dirty.insert(window);
        true
    }

    /// Whether `node` has a drawn control's state, for tests: a released node keeps none.
    pub fn holds_control_state(&self, node: NodeId) -> bool {
        self.form_drawn.borrow().contains_key(&node)
    }

    /// Whether the window shows a context menu, for tests and Esc.
    pub fn menu_open(&self, window: WindowId) -> bool {
        self.state.borrow().menus.contains_key(&window)
    }

    /// A backdrop consumed a press: the release that ends it is consumed too, even once the
    /// overlay closed under it, so nothing below sees half a click.
    pub(crate) fn swallow_release(&self, window: WindowId) {
        self.state.borrow_mut().swallow_up.insert(window);
    }

    /// One pointer event for the gesture recognizers of `window`, delivered as the host
    /// delivers its own.
    pub(crate) fn gesture_pointer(
        &self,
        window: WindowId,
        phase: nd_gesture::PointerPhase,
        at: Point,
        wake: &crate::backend::Wake,
        gpui_window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) {
        let outputs = {
            let state = self.state.borrow();
            let Some(root) = state.windows.get(&window).and_then(|w| w.root) else { return };
            let bounds = self.bounds.borrow();
            self.gestures.pointer(window, phase, at, (&state.nodes, &bounds, root))
        };
        let events = self.state.borrow().events.clone();
        let (queued, scrolled) = crate::gestures::deliver(&events, outputs);
        if queued {
            wake(cx);
        }
        if scrolled || self.gestures.needs_frame(window) {
            gpui_window.refresh();
        }
    }

    pub(crate) fn clear_swallow(&self, window: WindowId) {
        self.state.borrow_mut().swallow_up.remove(&window);
    }

    pub(crate) fn take_swallow(&self, window: WindowId) -> bool {
        self.state.borrow_mut().swallow_up.remove(&window)
    }

    pub(crate) fn take_actions(&self) -> Vec<(WindowId, Box<dyn gpui::Action>)> {
        std::mem::take(&mut self.state.borrow_mut().actions)
    }
}

/// [`ChromeServices`] on GPUI: drawn context menus in the overlay layer, drawn alerts and the
/// about panel ([`crate::dialogs`]). Menu bars fold into the command palette and file dialogs go
/// through the portal in the OS lanes; until then they answer with the cancel result.
pub struct GpuiChrome(GpuiBackend);

impl GpuiChrome {
    pub fn new(backend: GpuiBackend) -> Rc<Self> {
        Rc::new(Self(backend))
    }
}

impl ChromeServices for GpuiChrome {
    fn set_menus(&self, _cx: &mut nd_ui::App, _menus: Vec<Menu>) {}

    fn show_context_menu(
        &self,
        _cx: &mut nd_ui::App,
        window: WindowId,
        node: NodeId,
        x: f32,
        y: f32,
        menu: ContextMenu,
    ) {
        let origin = self.0.bounds(node).map_or(Point::default(), |b| b.origin);
        let at = Point { x: origin.x + x, y: origin.y + y };
        let replaced = {
            let mut state = self.0.state.borrow_mut();
            state.dirty.insert(window);
            state.menus.insert(
                window,
                MenuOverlay {
                    node,
                    at,
                    above: menu.above,
                    content: MenuContent::Actions(menu.items),
                    open: Vec::new(),
                },
            )
        };
        if let Some(old) = replaced {
            old.dismissed();
        }
    }

    fn alert(&self, _cx: &mut nd_ui::App, window: WindowId, alert: Alert, done: AlertCallback) {
        self.0.show_alert(window, alert, done);
    }

    fn about(&self, _cx: &mut nd_ui::App, window: Option<WindowId>, about: AboutPanel) {
        self.0.show_about(window, about);
    }

    fn open_dialog(&self, cx: &mut nd_ui::App, _window: WindowId, _dialog: OpenDialog, done: OpenCallback) {
        done(None::<Vec<PathBuf>>, cx);
    }

    fn save_dialog(&self, cx: &mut nd_ui::App, _window: WindowId, _dialog: SaveDialog, done: SaveCallback) {
        done(None, cx);
    }
}

/// Seconds and curve an overlay takes to leave, by look: WinUI's "gentle exit"
/// (cubic-bezier(1, 0, 1, 1), 167 ms); elsewhere the look's fast motion, easing in.
pub(crate) fn exit_timing(theme: &nd_theme::Theme) -> (f64, fn(f32) -> f32) {
    if theme.preset == nd_theme::Preset::Fluent {
        (0.167, |t| crate::forms::look::cubic_bezier([1., 0., 1., 1.], t))
    } else {
        (theme.motion.fast.as_secs_f64(), |t| t * t * t)
    }
}

/// An overlay's subtree and where it hung, as last drawn.
#[derive(Clone)]
pub(crate) struct Snapshot {
    pub window: nd_core::WindowId,
    pub nodes: std::collections::HashMap<NodeId, Node>,
    pub anchor: Option<Rect>,
    /// When it closed; `None` while it is open.
    pub left: Option<nd_core::time::Instant>,
}

/// Open overlays' snapshots and the closed ones still animating out.
#[derive(Clone, Default)]
pub(crate) struct Exits(Rc<RefCell<std::collections::HashMap<NodeId, Snapshot>>>);

impl Exits {
    pub fn seen(
        &self,
        id: NodeId,
        window: nd_core::WindowId,
        nodes: std::collections::HashMap<NodeId, Node>,
        anchor: Option<Rect>,
    ) {
        self.0.borrow_mut().insert(id, Snapshot { window, nodes, anchor, left: None });
    }

    /// `id` closed: it starts leaving, unless motion is off (then it is forgotten). Returns its
    /// window when it leaves.
    pub fn leave(&self, id: NodeId, reduce: bool) -> Option<nd_core::WindowId> {
        let mut map = self.0.borrow_mut();
        let snapshot = map.get_mut(&id)?;
        if snapshot.left.is_some() {
            return None;
        }
        if reduce {
            map.remove(&id);
            return None;
        }
        snapshot.left = Some(nd_core::time::Instant::now());
        Some(snapshot.window)
    }

    /// The overlays of `window` animating out, with how long ago each closed. Ones done after
    /// `duration` are dropped.
    pub fn leaving(&self, window: nd_core::WindowId, duration: f64) -> Vec<(NodeId, Snapshot, f64)> {
        let mut map = self.0.borrow_mut();
        let now = nd_core::time::Instant::now();
        map.retain(|_, s| s.left.is_none_or(|at| now.duration_since(at).as_secs_f64() < duration));
        map.iter()
            .filter(|(_, s)| s.window == window)
            .filter_map(|(id, s)| s.left.map(|at| (*id, s.clone(), now.duration_since(at).as_secs_f64())))
            .collect()
    }
}

impl Build<'_> {
    /// `id` and every node under it.
    fn subtree(&self, id: NodeId) -> std::collections::HashMap<NodeId, Node> {
        let mut out = std::collections::HashMap::new();
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            if let Some(node) = self.nodes.get(&n) {
                stack.extend(node.children.iter().copied());
                out.insert(n, node.clone());
            }
        }
        out
    }
}

/// A submenu beside its row: past the menu's trailing edge, or past its leading edge when the
/// window has no room there, never over the menu itself. Its first row lines up with the row it
/// opens from, and it stays inside the window vertically.
struct Beside {
    /// From the row's sides to the menu's outer edges.
    edge: Pixels,
    child: AnyElement,
}

impl Beside {
    /// How far the submenu reaches back over the menu's edge.
    const OVERLAP: Pixels = px(2.);
    /// From the submenu's top to its first row's top: the list's padding and border.
    const LEAD: Pixels = px(5.);
    const MARGIN: Pixels = px(8.);

    /// The submenu's origin for a row at `row`, in a window of `viewport`.
    fn place(
        row: Bounds<Pixels>,
        size: gpui::Size<Pixels>,
        edge: Pixels,
        viewport: gpui::Size<Pixels>,
    ) -> gpui::Point<Pixels> {
        let (left, right) = (Self::MARGIN, viewport.width - Self::MARGIN);
        let after = row.right() + edge - Self::OVERLAP;
        let before = row.left() - edge + Self::OVERLAP - size.width;
        let x = if after + size.width <= right {
            after
        } else if before >= left {
            before
        } else if right - row.right() >= row.left() {
            // Room on neither side: the wider one, kept inside the window.
            (right - size.width).max(left).min(after)
        } else {
            left.max(before)
        };
        let y = (row.top() - Self::LEAD).min(viewport.height - Self::MARGIN - size.height).max(Self::MARGIN);
        point(x, y)
    }
}

impl gpui::Element for Beside {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    // Laid out as the row's own box; the submenu is laid out on its own in prepaint, at its
    // natural size, so the row's width does not squeeze it.
    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> (gpui::LayoutId, ()) {
        let mut style = gpui::Style { position: gpui::Position::Absolute, ..Default::default() };
        style.inset = gpui::Edges::all(px(0.).into());
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) {
        let size = self.child.layout_as_root(
            gpui::Size::new(gpui::AvailableSpace::MaxContent, gpui::AvailableSpace::MaxContent),
            window,
            cx,
        );
        let inset = window.client_inset().unwrap_or_default();
        let mut viewport = window.viewport_size();
        viewport.width -= inset * 2.;
        viewport.height -= inset * 2.;
        let row = Bounds { origin: bounds.origin - point(inset, inset), size: bounds.size };
        let at = Self::place(row, size, self.edge, viewport) + point(inset, inset);
        self.child.prepaint_at(at, window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) {
        self.child.paint(window, cx);
    }
}

impl IntoElement for Beside {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use nd_gpui::gpui::{Bounds, point, px, size};

    use super::Beside;

    const WINDOW: nd_gpui::gpui::Size<nd_gpui::gpui::Pixels> = size(px(800.), px(600.));

    #[test]
    fn a_submenu_opens_past_the_trailing_edge_when_it_fits() {
        let row = Bounds { origin: point(px(100.), px(200.)), size: size(px(180.), px(28.)) };
        let at = Beside::place(row, size(px(200.), px(120.)), px(1.), WINDOW);
        assert_eq!(at, point(px(279.), px(195.)));
    }

    #[test]
    fn a_submenu_with_no_room_after_opens_before_the_menu_not_over_it() {
        let row = Bounds { origin: point(px(560.), px(200.)), size: size(px(180.), px(28.)) };
        let at = Beside::place(row, size(px(200.), px(120.)), px(1.), WINDOW);
        assert_eq!(at, point(px(561.) - px(200.), px(195.)));
        assert!(at.x + px(200.) <= row.left() + px(2.), "it ends at the menu's edge, overlapping by 2 px at most");
    }

    #[test]
    fn a_submenu_near_the_bottom_stays_in_the_window() {
        let row = Bounds { origin: point(px(100.), px(560.)), size: size(px(180.), px(28.)) };
        let at = Beside::place(row, size(px(200.), px(120.)), px(1.), WINDOW);
        assert_eq!(at.y, px(600. - 8. - 120.));
    }
}
