//! The overlay layer on GPUI (ARCHITECTURE.md 6.3): every overlay draws in GPUI's deferred layer,
//! above all content of its window and above every native surface element, nested overlays above
//! their parents. Input is captured there:
//! - The overlay's content occludes what is under it, so presses, scrolls and hover never reach
//!   a button or surface below.
//! - A non-passive overlay lays a transparent backdrop over the window. A press on it is
//!   consumed; on a light-dismiss overlay it also asks the overlay to close. A modal backdrop
//!   blocks scrolling outside too; a light-dismiss one lets scrolling through to content outside
//!   the overlay, as AppKit does.
//! - Passive overlays (tooltips) take no input.
//!
//! Context menus are a drawn menu in the same layer, opened through [`GpuiChrome`]; items
//! dispatch their action from the window's focused node, as a key binding would.
//!
//! Native surfaces composited by the OS above GPUI's base layer (WebView2, WPE subsurfaces) need
//! gpui-fast's overlay composition surface to stay under this layer; that arrives with the M8
//! engines. Today's GPU test surface draws inside GPUI, so the deferred layer covers it already.

use std::path::PathBuf;
use std::rc::Rc;

use nd_components::services::{AlertCallback, ChromeServices, OpenCallback, SaveCallback};
use nd_components::{Alert, AlertResponse, ContextMenu, Menu, MenuItem, OpenDialog, SaveDialog};
use nd_core::{NodeId, Point, WindowId};
use nd_gpui::gpui::{
    self, Anchor, AnyElement, Bounds, DispatchPhase, ElementId, HitboxBehavior, InteractiveElement, IntoElement,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Refineable, ScrollWheelEvent,
    StatefulInteractiveElement, Styled, anchored, canvas, deferred, div, fill, hsla, point, px,
};
use nd_ui::{DismissCause, Dismissed, Modality, OverlayKind, OverlayProps, Placement, SurfaceProps};

use crate::backend::{GpuiBackend, Node};
use crate::render::{Build, Tracked};

/// Stacking inside GPUI's deferred layer, one order for every overlay kind: toasts lowest (they
/// never block), framework panels (the command palette) and overlays above them, nested overlays
/// above their parents, the context menu on top.
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
    pub items: Vec<MenuItem>,
}

impl Build<'_> {
    fn overlay_depth(&self, id: NodeId) -> usize {
        let mut depth = 0;
        let mut at = self.nodes.get(&id).and_then(|n| n.parent);
        while let Some(n) = at {
            let node = &self.nodes[&n];
            if node.props.downcast_ref::<OverlayProps>().is_some() {
                depth += 1;
            }
            at = node.parent;
        }
        depth
    }

    pub(crate) fn overlay(&self, id: NodeId, node: &Node, window: &mut gpui::Window) -> AnyElement {
        let props = node.props.downcast_ref::<OverlayProps>().expect("Overlay node needs OverlayProps");
        let theme = self.theme();
        let passive = props.modality == Modality::Passive;
        let mut content = div().id(ElementId::Integer(id.0));
        content.style().refine(&props.style);
        content.style().position = Some(gpui::Position::Relative);
        // The app's own background, or the theme's overlay surface; content builds over it.
        let styled = props.style.background.as_ref().and_then(|f| f.color()).and_then(|b| b.as_solid());
        let fill = styled.unwrap_or(theme.surface.overlay);
        if styled.is_none() {
            content =
                content.bg(fill).border_1().border_color(theme.border.subtle).rounded(theme.radius.lg).shadow_md();
        }
        let children: Vec<AnyElement> = self.painted(id, Some(fill), Some(theme.text.primary), "overlay", || {
            node.children.iter().map(|c| self.node(*c, window)).collect()
        });
        if !passive {
            content = content
                .occlude()
                .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation());
        }
        let content = Tracked::new(id, content.children(children).into_any_element(), self.backend.bounds.clone());

        let viewport = window.viewport_size();
        let positioned = if props.kind == OverlayKind::Panel {
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
            if let Some(a) = node.parent.and_then(|p| self.backend.bounds(p)) {
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
            let mut backdrop = div().id(ElementId::Name(format!("nd-backdrop-{}", id.0).into()));
            backdrop = if light { backdrop.block_mouse_except_scroll() } else { backdrop.occlude() };
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
            anchored().position(point(px(0.), px(0.))).child(backdrop).into_any_element()
        };
        deferred(layer).with_priority(PANEL_PRIORITY + 1 + self.overlay_depth(id)).into_any_element()
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
        let mut list = div()
            .id("nd-context-menu")
            .flex()
            .flex_col()
            .min_w(px(180.))
            .py(px(4.))
            .bg(surface)
            .border_1()
            .border_color(theme.border.subtle)
            .rounded(theme.radius.md)
            .text_color(theme.text.primary)
            .shadow_md()
            .occlude()
            .on_any_mouse_down(|_, _, cx| cx.stop_propagation());
        for (i, item) in menu.items.iter().enumerate() {
            list = list.child(match item {
                MenuItem::Separator => {
                    marks.fill("menu separator", theme.separator, &[surface]);
                    div().h(px(1.)).my(px(4.)).bg(theme.separator).into_any_element()
                }
                MenuItem::Action { label, action, enabled, .. } => {
                    let backend = self.backend.clone();
                    let wake = self.wake.clone();
                    let action = action.boxed_clone();
                    let mut row = div().id(ElementId::Integer(i as u64)).px(px(12.)).py(px(4.)).child(label.clone());
                    if *enabled {
                        marks.text(label, theme.text.primary, &[surface], body, regular);
                        let hover = theme.hover(surface);
                        row = row.hover(move |s| s.bg(hover)).on_click(move |_, _, cx| {
                            backend.choose(window_id, action.boxed_clone());
                            wake(cx);
                        });
                    } else {
                        marks.text(label, theme.text.tertiary, &[surface], body, regular);
                        row = row.text_color(theme.text.tertiary);
                    }
                    row.into_any_element()
                }
                MenuItem::Submenu(sub) => {
                    marks.text(&sub.title, theme.text.primary, &[surface], body, regular);
                    div().px(px(12.)).py(px(4.)).child(format!("{} \u{203a}", sub.title)).into_any_element()
                }
                MenuItem::Role(_) => div().into_any_element(),
            });
        }
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
                anchored().position(point(px(menu.at.x), px(menu.at.y))).snap_to_window_with_margin(px(8.)).child(list),
            );
        deferred(anchored().position(point(px(0.), px(0.))).child(backdrop))
            .with_priority(MENU_PRIORITY)
            .into_any_element()
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
        let mut state = self.state.borrow_mut();
        let closed = state.menus.remove(&window).is_some();
        if closed {
            state.dirty.insert(window);
        }
        closed
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

/// [`ChromeServices`] on GPUI: drawn context menus in the overlay layer. Menu bars fold into the
/// command palette and dialogs go through the portal in the OS lanes; until then they answer
/// with the cancel result.
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
        let mut state = self.0.state.borrow_mut();
        state
            .menus
            .insert(window, MenuOverlay { node, at: Point { x: origin.x + x, y: origin.y + y }, items: menu.items });
        state.dirty.insert(window);
    }

    fn alert(&self, cx: &mut nd_ui::App, _window: WindowId, _alert: Alert, done: AlertCallback) {
        done(AlertResponse(0), cx);
    }

    fn open_dialog(&self, cx: &mut nd_ui::App, _window: WindowId, _dialog: OpenDialog, done: OpenCallback) {
        done(None::<Vec<PathBuf>>, cx);
    }

    fn save_dialog(&self, cx: &mut nd_ui::App, _window: WindowId, _dialog: SaveDialog, done: SaveCallback) {
        done(None, cx);
    }
}
