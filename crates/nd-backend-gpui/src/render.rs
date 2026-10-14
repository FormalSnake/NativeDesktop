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

use nd_components::{ContextMenuRequested, GlassProps, ScaffoldProps, SubtreeScope};
use nd_core::{NodeId, Rect};
use nd_gpui::gpui::{
    self, AnyElement, App, Bounds, Decorations, ElementId, FontFeatures, FontWeight, GlobalElementId, Hsla,
    InspectorElementId, InteractiveElement, IntoElement, LayoutId, MouseButton, MouseDownEvent, ParentElement, Pixels,
    Refineable, StatefulInteractiveElement, StyleRefinement, Styled, WindowControlArea, div, px,
};
use nd_theme::color::{over, with_alpha};
use nd_theme::{Theme, ThemeRefinement, ThemeRegistry};
use nd_ui::backend::NodeKind;
use nd_ui::overlay::Overlay;
use nd_ui::semantics::{Paint, PaintKind};
use nd_ui::surface::NativeSurface;
use nd_ui::{
    Bezel, ButtonVariant, ClickEvent, DivProps, Scope, TextProps, TitlebarInsets, WindowControls, WindowControlsProps,
    with_titlebar_insets,
};

use crate::backend::{GpuiBackend, Node, Wake, is_button};
use crate::forms::glyph::Glyph;
use crate::os::{ChromeContext, OsHooks};

/// Records the bounds GPUI laid an element out at.
pub(crate) struct Tracked {
    node: NodeId,
    inner: AnyElement,
    bounds: Rc<RefCell<HashMap<NodeId, Rect>>>,
}

impl Tracked {
    pub(crate) fn new(node: NodeId, inner: AnyElement, bounds: Rc<RefCell<HashMap<NodeId, Rect>>>) -> Self {
        Self { node, inner, bounds }
    }
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

/// The colours the drawn chrome and controls use, taken from the theme in effect.
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
    pub fn of(theme: &Theme) -> Self {
        let control = theme.surface.raised;
        Self {
            background: theme.surface.base,
            text: theme.text.primary,
            control,
            control_hover: theme.hover(control),
            control_active: theme.pressed(control),
            border: theme.border.subtle,
            close_hover: theme.danger,
        }
    }
}

/// The theme, text colour and background in effect while a subtree builds.
#[derive(Clone)]
struct Ink {
    theme: Rc<Theme>,
    /// The subtree refinements so far, composed, for the next nested scope to stack on.
    refinement: Option<ThemeRefinement>,
    scopes: Vec<Scope>,
    text: Hsla,
    /// Opaque.
    background: Hsla,
    /// The surface a scope that flipped the scheme leaves for its node to paint.
    surface: Option<Hsla>,
}

/// What a text run is drawn at, for its paint record.
#[derive(Clone, Copy)]
pub(crate) struct Run {
    pub size: f32,
    pub weight: f32,
}

impl Run {
    pub const BODY: Run = Run { size: REM, weight: 400. };

    pub fn sized(size: f32) -> Run {
        Run { size, ..Run::BODY }
    }

    pub fn weight(self, weight: FontWeight) -> Run {
        Run { weight: weight.0, ..self }
    }
}

/// GPUI's default rem, which text with no size of its own is drawn at.
const REM: f32 = 16.;

fn solid(style: &StyleRefinement) -> Option<Hsla> {
    style.background.as_ref().and_then(|f| f.color()).and_then(|b| b.as_solid()).filter(|c| c.a > 0.)
}

fn run_of(style: &StyleRefinement) -> Run {
    Run {
        size: style.text.font_size.map_or(REM, |s| s.to_pixels(px(REM)).as_f32()),
        weight: style.text.font_weight.map_or(400., |w| w.0),
    }
}

fn scope_of(props: &nd_ui::backend::Props) -> Option<&SubtreeScope> {
    if let Some(d) = props.downcast_ref::<DivProps>() {
        return d.scope.as_ref().and_then(|s| s.get::<SubtreeScope>());
    }
    if let Some(g) = props.downcast_ref::<GlassProps>() {
        return Some(&g.scope);
    }
    props.downcast_ref::<ScaffoldProps>().map(|s| &s.scope)
}

pub(crate) struct Build<'a> {
    pub backend: &'a GpuiBackend,
    pub nodes: &'a HashMap<NodeId, Node>,
    pub insets: TitlebarInsets,
    pub wake: Wake,
    pub hooks: &'a Rc<dyn OsHooks>,
    pub window_id: nd_core::WindowId,
    /// The frame's display time in seconds, which drawn animations sample.
    pub time: f64,
    registry: &'a ThemeRegistry,
    ink: RefCell<Vec<Ink>>,
    paints: RefCell<Vec<Paint>>,
}

impl<'a> Build<'a> {
    pub fn new(
        backend: &'a GpuiBackend,
        nodes: &'a HashMap<NodeId, Node>,
        registry: &'a ThemeRegistry,
        insets: TitlebarInsets,
        wake: Wake,
        hooks: &'a Rc<dyn OsHooks>,
        window_id: nd_core::WindowId,
    ) -> Self {
        let theme = registry.window_theme(window_id).clone();
        let ink = Ink {
            text: theme.text.primary,
            background: theme.surface.base,
            theme: Rc::new(theme),
            refinement: None,
            scopes: Vec::new(),
            surface: None,
        };
        Self {
            backend,
            nodes,
            insets,
            wake,
            hooks,
            window_id,
            time: 0.,
            registry,
            ink: RefCell::new(vec![ink]),
            paints: RefCell::default(),
        }
    }

    /// Everything painted while building, in paint order.
    pub fn take_paints(&self) -> Vec<Paint> {
        self.paints.take()
    }

    fn top(&self) -> Ink {
        self.ink.borrow().last().cloned().expect("the window's ink is never popped")
    }

    /// The theme in effect where the build is.
    pub fn theme(&self) -> Rc<Theme> {
        self.top().theme
    }

    /// The colour text inherits where the build is.
    pub fn ink_text(&self) -> Hsla {
        self.top().text
    }

    /// The opaque colour under what the build is at.
    pub(crate) fn backdrop(&self) -> Hsla {
        self.top().background
    }

    pub fn look(&self) -> Look {
        Look::of(&self.top().theme)
    }

    /// Records a fill under `node` and builds what `inside` returns over it, with `text` as
    /// the colour text inside inherits.
    pub fn painted<R>(
        &self,
        node: NodeId,
        fill: Option<Hsla>,
        text: Option<Hsla>,
        what: &str,
        inside: impl FnOnce() -> R,
    ) -> R {
        let mut ink = self.top();
        ink.surface = None;
        if let Some(fill) = fill.filter(|c| c.a > 0.) {
            self.paints.borrow_mut().push(Paint {
                node: Some(node),
                kind: PaintKind::Fill,
                color: fill,
                background: ink.background,
                scopes: ink.scopes.clone(),
                label: what.into(),
            });
            ink.background = over(fill, ink.background);
        }
        if let Some(text) = text {
            ink.text = text;
        }
        self.ink.borrow_mut().push(ink);
        let out = inside();
        self.ink.borrow_mut().pop();
        out
    }

    /// Records a text run of `node`: `color` or the inherited colour, over `fill` when the run
    /// has a background of its own.
    pub fn text_run(&self, node: NodeId, text: &str, run: Run, color: Option<Hsla>, fill: Option<Hsla>) -> Hsla {
        let ink = self.top();
        let background = fill.map_or(ink.background, |f| over(f, ink.background));
        let color = color.unwrap_or(ink.text);
        self.paints.borrow_mut().push(Paint {
            node: Some(node),
            kind: PaintKind::Text { size: run.size, weight: run.weight },
            color,
            background,
            scopes: ink.scopes.clone(),
            label: text.into(),
        });
        color
    }

    /// Records what a drawn widget painted, each over its own fills and then what the build
    /// has painted beneath the widget.
    pub(crate) fn record_marks(&self, node: NodeId, marks: Vec<crate::nav::Mark>) {
        let ink = self.top();
        let mut paints = self.paints.borrow_mut();
        for mark in marks {
            let background = mark.under.iter().fold(ink.background, |b, f| over(*f, b));
            let kind = match mark.text {
                Some((size, weight)) => PaintKind::Text { size, weight },
                None => PaintKind::Fill,
            };
            paints.push(Paint {
                node: Some(node),
                kind,
                color: mark.color,
                background,
                scopes: ink.scopes.clone(),
                label: mark.label,
            });
        }
    }

    /// Enters the theme of `node`'s subtree scope, if it has one, for what `inside` builds. A
    /// scope that changes the scheme leaves its surface for the node to paint, so text inside
    /// reads on it.
    fn scoped<R>(&self, node: &Node, inside: impl FnOnce() -> R) -> R {
        let Some(scope) = scope_of(&node.props) else { return inside() };
        let Some(refinement) = &scope.theme else { return inside() };
        let outer = self.top();
        let composed = match &outer.refinement {
            Some(o) => refinement.over(o),
            None => refinement.clone(),
        };
        let theme = self.registry.resolve_subtree(Some(self.window_id), &composed);
        let mut scopes = outer.scopes.clone();
        scopes.push(Scope::new(scope.clone()));
        let surface = (theme.scheme != outer.theme.scheme).then_some(theme.surface.base);
        self.ink.borrow_mut().push(Ink {
            text: theme.text.primary,
            background: outer.background,
            theme: Rc::new(theme),
            refinement: Some(composed),
            scopes,
            surface,
        });
        let out = inside();
        self.ink.borrow_mut().pop();
        out
    }

    pub fn node(&self, id: NodeId, window: &mut gpui::Window, cx: &mut App) -> AnyElement {
        let node = &self.nodes[&id];
        let element = self.scoped(node, || self.element(id, node, window, cx));
        Tracked { node: id, inner: element, bounds: self.backend.bounds.clone() }.into_any_element()
    }

    fn element(&self, id: NodeId, node: &Node, window: &mut gpui::Window, cx: &mut App) -> AnyElement {
        let children = |w: &mut gpui::Window, cx: &mut App| -> Vec<AnyElement> {
            node.children.iter().map(|c| self.node(*c, w, cx)).collect()
        };
        match node.kind {
            NodeKind::Root => {
                let ink = self.top();
                self.painted(id, Some(ink.background), Some(ink.text), "window", || {
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .bg(ink.background)
                        .text_color(ink.text)
                        .children(children(window, cx))
                        .into_any_element()
                })
            }
            NodeKind::View => {
                let props = node.props.downcast_ref::<DivProps>().expect("View node needs DivProps");
                let own = solid(&props.style);
                let surface = own.is_none().then(|| self.top().surface).flatten();
                self.painted(id, own.or(surface), props.style.text.color, "view", || {
                    self.view(id, props, surface, children(window, cx), window)
                })
            }
            NodeKind::Text => {
                let props = node.props.downcast_ref::<TextProps>().expect("Text node needs TextProps");
                let mut d = div();
                d.style().refine(&props.style);
                if props.tabular_nums {
                    d.style().text.font_features = Some(FontFeatures(Arc::new(vec![("tnum".into(), 1)])));
                }
                let opacity = props.style.opacity.unwrap_or(1.);
                // Never left to GPUI's default (black): every run carries the colour it is audited at.
                let color = props.style.text.color.unwrap_or(self.top().text);
                let fill = solid(&props.style);
                self.text_run(id, &props.text, run_of(&props.style), Some(with_alpha(color, color.a * opacity)), fill);
                d.style().text.color = Some(color);
                d.child(props.text.clone()).into_any_element()
            }
            NodeKind::Widget(_) if is_button(node.kind) => self.button(id, node),
            NodeKind::Widget(w) if w.type_id == std::any::TypeId::of::<WindowControls>() => {
                let props = node.props.downcast_ref::<WindowControlsProps>().expect("WindowControlsProps");
                self.window_controls(props, window)
            }
            // Its bounds are its content's, which the overlay layer places and records after this
            // node's placeholder.
            NodeKind::Widget(w) if w.type_id == std::any::TypeId::of::<Overlay>() => self.overlay(id, node, window, cx),
            NodeKind::Widget(w) if w.type_id == std::any::TypeId::of::<NativeSurface>() => self.surface(id, node),
            NodeKind::Widget(w) if w.type_id == std::any::TypeId::of::<nd_virtual::ListWidget>() => {
                self.virtual_list(id, node, window, cx)
            }
            NodeKind::Widget(w) if let Some(intent) = self.intent(id, w, node, window, cx) => intent,
            NodeKind::Widget(w) if let Some(custom) = crate::custom::lookup(w.type_id) => {
                custom.element(&crate::custom::CustomCx::new(self, id, &node.props), window)
            }
            NodeKind::Canvas | NodeKind::Image | NodeKind::Widget(_) => {
                // Not mapped yet (canvas and images land with M4 and M6); keeps its layout box.
                div().into_any_element()
            }
        }
    }

    fn virtual_list(&self, id: NodeId, node: &Node, window: &mut gpui::Window, cx: &mut App) -> AnyElement {
        let props = node.props.downcast_ref::<nd_virtual::ListProps>().expect("ListProps").clone();
        let emit = self.backend.state.borrow().events.emitter(id);
        let look = self.look();
        let selection = look.control_active;
        let rows = node
            .children
            .iter()
            .filter_map(|c| {
                let tag = self
                    .nodes
                    .get(c)?
                    .props
                    .downcast_ref::<DivProps>()?
                    .scope
                    .as_ref()?
                    .get::<nd_virtual::RowTag>()?
                    .clone();
                let fill = props.selected.contains(&tag.row).then_some(selection);
                let element = self.painted(*c, fill, None, "selected row", || {
                    if tag.expandable {
                        self.painted(*c, Some(self.top().text), None, "disclosure", || ());
                    }
                    self.node(*c, window, cx)
                });
                Some(crate::virtual_list::Row { tag, node: *c, element })
            })
            .collect();
        let header = match &props.kind {
            nd_virtual::ListKind::Table(columns) => {
                for column in columns {
                    self.text_run(id, &column.title, Run::BODY, None, None);
                }
                Some(crate::virtual_list::header(columns, &look, self.top().text, emit.clone(), self.wake.clone()))
            }
            _ => None,
        };
        crate::virtual_list::VirtualList {
            node: id,
            props,
            rows,
            header,
            emit,
            wake: self.wake.clone(),
            presenters: self.backend.lists.clone(),
            gestures: self.backend.gestures.clone(),
            window_id: self.window_id,
            selection_color: selection,
            disclosure_color: self.top().text,
        }
        .into_any_element()
    }

    fn view(
        &self,
        id: NodeId,
        props: &DivProps,
        surface: Option<Hsla>,
        children: Vec<AnyElement>,
        window: &mut gpui::Window,
    ) -> AnyElement {
        let mut style =
            if props.titlebar_insets { with_titlebar_insets(&props.style, self.insets) } else { props.style.clone() };
        if let Some(surface) = surface {
            style.background = Some(surface.into());
        }
        // Spelled out on every view, so text under a subtree scope takes the scope's colour.
        style.text.color = Some(self.top().text);
        let menu = props.scope.as_ref().and_then(|s| s.get::<SubtreeScope>()).is_some_and(|s| s.context_menu);
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
            if menu {
                let emit = self.backend.state.borrow().events.emitter(id);
                let (wake, bounds) = (self.wake.clone(), self.backend.bounds.clone());
                d = d.on_mouse_down(MouseButton::Right, move |event: &MouseDownEvent, _, cx| {
                    let origin = bounds.borrow().get(&id).map_or(nd_core::Point::default(), |b| b.origin);
                    let (x, y) = (event.position.x.as_f32() - origin.x, event.position.y.as_f32() - origin.y);
                    emit.emit(ContextMenuRequested { node: id, x, y });
                    wake(cx);
                    cx.stop_propagation();
                });
            }
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
        let look = self.look();
        let theme = self.theme();
        // An icon-only button draws its glyph; one with no glyph here shows its tooltip.
        let glyph = if props.label.is_empty() { props.icon.as_deref().and_then(Glyph::named) } else { None };
        let label = if props.label.is_empty() {
            props.tooltip.clone().or_else(|| props.icon.clone()).unwrap_or_default()
        } else {
            props.label.clone()
        };
        let name = if props.label.is_empty() { label.clone() } else { props.label.clone() };
        let accent = props.variant == ButtonVariant::Accent;
        let (fill, ink) = if accent { (theme.accent, theme.text.on_accent) } else { (look.control, look.text) };
        let (fill, ink) = if props.enabled { (fill, ink) } else { (theme.disabled(fill), theme.text.tertiary) };
        // Borderless: no bezel until the pointer is over it.
        let bezel = props.bezel != Bezel::Borderless || accent;
        let rest = bezel.then_some(fill);
        self.painted(id, rest, None, "button", || {
            if glyph.is_some() {
                self.record_marks(
                    id,
                    vec![crate::nav::Mark { label: label.to_string(), color: ink, under: vec![], text: None }],
                );
            } else {
                self.text_run(id, &label, Run::BODY, Some(ink), None);
            }
        });
        let mut button = div()
            .id(ElementId::Integer(id.0))
            .role(gpui::Role::Button)
            .aria_label(name)
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .text_color(ink)
            // Inside a drag region the button keeps its clicks and never starts a window move.
            .occlude();
        button = match glyph {
            Some(g) => button.size(px(28.)).child(crate::forms::glyph::glyph(g, px(16.), ink, px(1.75))),
            None => button.px(px(12.)).py(px(4.)).child(label),
        };
        if bezel {
            button = button.border_1().border_color(look.border).bg(fill);
        }
        if !props.enabled {
            return button.into_any_element();
        }
        let (hover, active) = (theme.hover(fill), theme.pressed(fill));
        button
            .hover(move |s| s.bg(hover))
            .active(move |s| s.bg(active))
            .on_click(move |_, _, cx| {
                emit.emit(ClickEvent);
                wake(cx);
            })
            .into_any_element()
    }

    /// The OS's own controls where it draws them (macOS traffic lights, server decorations);
    /// drawn buttons only under client-side decorations, for the actions the compositor offers.
    fn window_controls(&self, props: &WindowControlsProps, window: &mut gpui::Window) -> AnyElement {
        let cx = ChromeContext { window: self.window_id, at: props.at, look: self.look() };
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
    let mut row = div().absolute().top_0().right_0().flex().flex_row().text_color(look.text);
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
