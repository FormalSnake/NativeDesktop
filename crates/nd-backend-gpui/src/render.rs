//! Builds a window's GPUI elements from the backend's retained copy of its tree. Called from the
//! window's GPUI view while GPUI draws; GPUI lays the result out with its own Taffy, from the same
//! `StyleRefinement`s `nd-ui` lays out with on retained backends.
//!
//! Every node is wrapped in [`Tracked`], which records the bounds GPUI gave it, so probes and
//! semantic actions see what was painted.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use nd_core::{NodeId, Rect};
use nd_gpui::gpui::{
    self, AnyElement, App, Bounds, Decorations, ElementId, FontFeatures, GlobalElementId, Hsla, InspectorElementId,
    InteractiveElement, IntoElement, LayoutId, MouseButton, MouseDownEvent, ParentElement, Pixels, Refineable,
    StatefulInteractiveElement, Styled, WindowAppearance, WindowControlArea, div, hsla, px,
};
use nd_ui::backend::NodeKind;
use nd_ui::{
    ClickEvent, DivProps, TextProps, TitlebarInsets, WindowControls, WindowControlsProps, with_titlebar_insets,
};

use crate::backend::{GpuiBackend, Node, Wake, is_button};
use crate::os::{ChromeContext, OsHooks};

/// Records the bounds GPUI laid an element out at.
pub(crate) struct Tracked {
    node: NodeId,
    inner: AnyElement,
    bounds: Rc<RefCell<HashMap<NodeId, Rect>>>,
}

impl IntoElement for Tracked {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl gpui::Element for Tracked {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut gpui::Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.inner.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut gpui::Window,
        cx: &mut App,
    ) {
        let rect = Rect::new(
            bounds.origin.x.as_f32(),
            bounds.origin.y.as_f32(),
            bounds.size.width.as_f32(),
            bounds.size.height.as_f32(),
        );
        self.bounds.borrow_mut().insert(self.node, rect);
        self.inner.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut gpui::Window,
        cx: &mut App,
    ) {
        self.inner.paint(window, cx);
    }
}

/// The plain GPUI look until `nd-theme` (M3): light or dark from the window's appearance.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub background: Hsla,
    pub text: Hsla,
    pub control: Hsla,
    pub control_hover: Hsla,
    pub control_active: Hsla,
    pub border: Hsla,
    pub close_hover: Hsla,
}

impl Look {
    pub fn of(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self {
                background: hsla(0., 0., 0.12, 1.),
                text: hsla(0., 0., 0.93, 1.),
                control: hsla(0., 0., 0.22, 1.),
                control_hover: hsla(0., 0., 0.27, 1.),
                control_active: hsla(0., 0., 0.32, 1.),
                border: hsla(0., 0., 0.32, 1.),
                close_hover: hsla(0., 0.72, 0.48, 1.),
            },
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self {
                background: hsla(0., 0., 0.98, 1.),
                text: hsla(0., 0., 0.1, 1.),
                control: hsla(0., 0., 0.92, 1.),
                control_hover: hsla(0., 0., 0.87, 1.),
                control_active: hsla(0., 0., 0.82, 1.),
                border: hsla(0., 0., 0.8, 1.),
                close_hover: hsla(0., 0.72, 0.48, 1.),
            },
        }
    }
}

pub(crate) struct Build<'a> {
    pub backend: &'a GpuiBackend,
    pub nodes: &'a HashMap<NodeId, Node>,
    pub insets: TitlebarInsets,
    pub look: Look,
    pub wake: Wake,
    pub hooks: &'a Rc<dyn OsHooks>,
    pub window_id: nd_core::WindowId,
}

impl Build<'_> {
    pub fn node(&self, id: NodeId, window: &mut gpui::Window) -> AnyElement {
        let node = &self.nodes[&id];
        let children =
            |w: &mut gpui::Window| -> Vec<AnyElement> { node.children.iter().map(|c| self.node(*c, w)).collect() };
        let element = match node.kind {
            NodeKind::Root => div()
                .size_full()
                .flex()
                .flex_col()
                .bg(self.look.background)
                .text_color(self.look.text)
                .children(children(window))
                .into_any_element(),
            NodeKind::View => {
                let props = node.props.downcast_ref::<DivProps>().expect("View node needs DivProps");
                self.view(id, props, children(window), window)
            }
            NodeKind::Text => {
                let props = node.props.downcast_ref::<TextProps>().expect("Text node needs TextProps");
                let mut d = div();
                d.style().refine(&props.style);
                if props.tabular_nums {
                    d.style().text.font_features = Some(FontFeatures(Arc::new(vec![("tnum".into(), 1)])));
                }
                d.child(props.text.clone()).into_any_element()
            }
            NodeKind::Widget(_) if is_button(node.kind) => self.button(id, node),
            NodeKind::Widget(w) if w.type_id == std::any::TypeId::of::<WindowControls>() => {
                let props = node.props.downcast_ref::<WindowControlsProps>().expect("WindowControlsProps");
                self.window_controls(props, window)
            }
            NodeKind::Widget(w) if let Some(intent) = self.intent(id, w, node, window) => intent,
            NodeKind::Canvas | NodeKind::Image | NodeKind::Widget(_) => {
                // Not mapped yet (canvas and images land with M4 and M6); keeps its layout box.
                div().into_any_element()
            }
        };
        Tracked { node: id, inner: element, bounds: self.backend.bounds.clone() }.into_any_element()
    }

    fn view(&self, id: NodeId, props: &DivProps, children: Vec<AnyElement>, window: &mut gpui::Window) -> AnyElement {
        let mut style =
            if props.titlebar_insets { with_titlebar_insets(&props.style, self.insets) } else { props.style.clone() };
        let gestures = &self.backend.gestures;
        let last_bounds = self.backend.bounds.borrow().get(&id).copied();
        if let Some(row) = props.gestures.row_swipe.map(|_| last_bounds.map_or(0., |b| b.size.width)) {
            gestures.row_config(self.window_id, id, &props.gestures, row);
        }
        let children = if props.gestures.scroll.any() {
            // nd scrolls it (momentum, rubber band); GPUI only clips.
            let offset = gestures.scroll_offset(self.window_id, id, &props.gestures, last_bounds);
            gestures.clear_content(id);
            for (axis, scrolls) in [
                (&mut style.overflow.x, props.gestures.scroll.horizontal),
                (&mut style.overflow.y, props.gestures.scroll.vertical),
            ] {
                if scrolls {
                    *axis = Some(gpui::Overflow::Hidden);
                }
            }
            children.into_iter().map(|c| gestures.shifted(id, offset, c)).collect()
        } else {
            children
        };
        if !props.drag_region {
            let mut d = div();
            d.style().refine(&style);
            return d.children(children).into_any_element();
        }
        let mut d = div().id(ElementId::Integer(id.0));
        d.style().refine(&style);
        let _ = window;
        d.window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, |event: &MouseDownEvent, window, _| {
                if event.click_count >= 2 {
                    // macOS follows AppleActionOnDoubleClick; elsewhere a double click maximizes.
                    if cfg!(target_os = "macos") { window.titlebar_double_click() } else { window.zoom_window() }
                } else {
                    start_window_move(window);
                }
            })
            .children(children)
            .into_any_element()
    }

    fn button(&self, id: NodeId, node: &Node) -> AnyElement {
        let props = GpuiBackend::button_label(&node.props);
        let emit = self.backend.state.borrow().events.emitter(id);
        let wake = self.wake.clone();
        let look = self.look;
        div()
            .id(ElementId::Integer(id.0))
            .role(gpui::Role::Button)
            .aria_label(props.label.clone())
            .flex()
            .items_center()
            .justify_center()
            .px(px(12.))
            .py(px(4.))
            .rounded(px(6.))
            .border_1()
            .border_color(look.border)
            .bg(look.control)
            .hover(move |s| s.bg(look.control_hover))
            .active(move |s| s.bg(look.control_active))
            // Inside a drag region the button keeps its clicks and never starts a window move.
            .occlude()
            // Icons arrive with `nd-icons` (M6); until then an icon-only button shows its tooltip.
            .child(if props.label.is_empty() {
                props.tooltip.clone().or_else(|| props.icon.clone()).unwrap_or_default()
            } else {
                props.label.clone()
            })
            .on_click(move |_, _, cx| {
                emit.emit(ClickEvent);
                wake(cx);
            })
            .into_any_element()
    }

    /// The OS's own controls where it draws them (macOS traffic lights, server decorations);
    /// drawn buttons only under client-side decorations, for the actions the compositor offers.
    fn window_controls(&self, props: &WindowControlsProps, window: &mut gpui::Window) -> AnyElement {
        let cx = ChromeContext { window: self.window_id, at: props.at, look: self.look };
        if let Some(custom) = self.hooks.window_controls(&cx, window) {
            return custom;
        }
        #[cfg(target_os = "macos")]
        if let Some(at) = props.at {
            window.set_traffic_light_position(gpui::point(px(at.x), px(at.y)));
        }
        match window.window_decorations() {
            Decorations::Server => div().absolute().into_any_element(),
            Decorations::Client { .. } if cfg!(target_os = "macos") => div().absolute().into_any_element(),
            Decorations::Client { .. } => drawn_controls(&cx, window),
        }
    }
}

/// Minimize, maximize and close at the window's trailing top corner, each only when the window
/// manager offers it. The OS lanes replace these through [`OsHooks::window_controls`].
pub fn drawn_controls(cx: &ChromeContext, window: &gpui::Window) -> AnyElement {
    let offered = window.window_controls();
    let look = cx.look;
    let button = |name: &'static str, glyph: &'static str, area: WindowControlArea, hover: Hsla| {
        div()
            .id(name)
            .role(gpui::Role::Button)
            .aria_label(name)
            .w(px(CONTROL_WIDTH))
            .h(px(CONTROL_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .window_control_area(area)
            .hover(move |s| s.bg(hover))
            .occlude()
            .child(glyph)
    };
    let mut row = div().absolute().top_0().right_0().flex().flex_row();
    if offered.minimize {
        row = row.child(
            button("minimize", "\u{2212}", WindowControlArea::Min, look.control_hover)
                .on_click(|_, window, _| window.minimize_window()),
        );
    }
    if offered.maximize {
        row = row.child(
            button("maximize", "\u{25a1}", WindowControlArea::Max, look.control_hover)
                .on_click(|_, window, _| window.zoom_window()),
        );
    }
    row.child(
        button("close", "\u{00d7}", WindowControlArea::Close, look.close_hover)
            .on_click(|_, window, _| window.remove_window()),
    )
    .into_any_element()
}

pub const CONTROL_WIDTH: f32 = 46.;
pub const CONTROL_HEIGHT: f32 = 32.;

/// The OS move a drag region starts. `ND_DIAG_WINDOW=1` prints `nd: window move` once the request
/// is out, so a tier 3 test moves the pointer only after the compositor can follow it.
pub(crate) fn start_window_move(window: &mut gpui::Window) {
    window.start_window_move();
    if std::env::var_os("ND_DIAG_WINDOW").is_some_and(|v| v == "1") {
        eprintln!("nd: window move");
    }
}
