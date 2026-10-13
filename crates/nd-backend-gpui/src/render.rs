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

use nd_components::{GlassProps, ScaffoldProps, SubtreeScope};
use nd_core::{NodeId, Rect};
use nd_gpui::gpui::{
    self, AnyElement, App, Bounds, Decorations, ElementId, FontFeatures, FontWeight, GlobalElementId, Hsla,
    InspectorElementId, InteractiveElement, IntoElement, LayoutId, MouseButton, MouseDownEvent, ParentElement, Pixels,
    Refineable, StatefulInteractiveElement, StyleRefinement, Styled, WindowControlArea, div, px,
};
use nd_theme::color::{over, with_alpha};
use nd_theme::{Theme, ThemeRefinement, ThemeRegistry};
use nd_ui::backend::NodeKind;
use nd_ui::semantics::{Paint, PaintKind};
use nd_ui::{
    ClickEvent, DivProps, Scope, TextProps, TitlebarInsets, WindowControls, WindowControlsProps, with_titlebar_insets,
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

    pub fn node(&self, id: NodeId, window: &mut gpui::Window) -> AnyElement {
        let node = &self.nodes[&id];
        let element = self.scoped(node, || self.element(id, node, window));
        Tracked { node: id, inner: element, bounds: self.backend.bounds.clone() }.into_any_element()
    }

    fn element(&self, id: NodeId, node: &Node, window: &mut gpui::Window) -> AnyElement {
        let children =
            |w: &mut gpui::Window| -> Vec<AnyElement> { node.children.iter().map(|c| self.node(*c, w)).collect() };
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
                        .children(children(window))
                        .into_any_element()
                })
            }
            NodeKind::View => {
                let props = node.props.downcast_ref::<DivProps>().expect("View node needs DivProps");
                let own = solid(&props.style);
                let surface = own.is_none().then(|| self.top().surface).flatten();
                self.painted(id, own.or(surface), props.style.text.color, "view", || {
                    self.view(id, props, surface, children(window), window)
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
            NodeKind::Widget(w) if let Some(intent) = self.intent(id, w, node, window) => intent,
            NodeKind::Canvas | NodeKind::Image | NodeKind::Widget(_) => {
                // Not mapped yet (canvas and images land with M4 and M6); keeps its layout box.
                div().into_any_element()
            }
        }
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
                    window.start_window_move();
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
        // Icons arrive with `nd-icons` (M6); until then an icon-only button shows its tooltip.
        let label = if props.label.is_empty() {
            props.tooltip.clone().or_else(|| props.icon.clone()).unwrap_or_default()
        } else {
            props.label.clone()
        };
        self.painted(id, Some(look.control), None, "button", || {
            self.text_run(id, &label, Run::BODY, Some(look.text), None);
        });
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
            .text_color(look.text)
            .hover(move |s| s.bg(look.control_hover))
            .active(move |s| s.bg(look.control_active))
            // Inside a drag region the button keeps its clicks and never starts a window move.
            .occlude()
            .child(label)
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
