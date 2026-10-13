//! Element descriptions: what `render` returns. Each becomes a retained node (or a view
//! boundary) through the reconciler; nothing here touches a backend.
//!
//! Identity (3.14): an element with an explicit id is keyed by it; every other element by the
//! `#[track_caller]` location of its constructor, plus an occurrence index among siblings with
//! the same location. `.when(..)` therefore never shifts a sibling onto another native node.

use std::any::{Any, TypeId};
use std::panic::Location;
use std::rc::Rc;

use nd_gpui::gpui::prelude::FluentBuilder;
use nd_gpui::gpui::{SharedString, StyleRefinement, Styled};

use crate::app::{AnyView, App, EntityId};
use crate::backend::{AnyProps, NodeKind, Props, WidgetKind};
use crate::window::Window;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Id(SharedString),
    Site(&'static Location<'static>),
    Entity(EntityId),
}

impl From<SharedString> for Key {
    fn from(id: SharedString) -> Self {
        Key::Id(id)
    }
}

impl From<&'static str> for Key {
    fn from(id: &'static str) -> Self {
        Key::Id(id.into())
    }
}

pub(crate) type Handler = Rc<dyn Fn(&dyn Any, &mut Window, &mut App)>;

pub(crate) struct NodeElement {
    pub kind: NodeKind,
    pub key: Key,
    pub props: Props,
    pub layout: StyleRefinement,
    /// The backend measures it (text, native widgets); containers are sized by Taffy alone.
    pub measured: bool,
    pub children: Vec<AnyElement>,
    pub handlers: Vec<(TypeId, Handler)>,
}

pub(crate) enum Element {
    Node(Box<NodeElement>),
    View(AnyView),
}

pub struct AnyElement(pub(crate) Element);

impl AnyElement {
    fn node(node: NodeElement) -> Self {
        AnyElement(Element::Node(Box::new(node)))
    }

    pub(crate) fn view(view: AnyView) -> Self {
        AnyElement(Element::View(view))
    }
}

pub trait IntoElement: Sized {
    fn into_any_element(self) -> AnyElement;
}

impl IntoElement for AnyElement {
    fn into_any_element(self) -> AnyElement {
        self
    }
}

impl IntoElement for SharedString {
    #[track_caller]
    fn into_any_element(self) -> AnyElement {
        text(self).into_any_element()
    }
}

impl IntoElement for &'static str {
    #[track_caller]
    fn into_any_element(self) -> AnyElement {
        text(self).into_any_element()
    }
}

impl IntoElement for String {
    #[track_caller]
    fn into_any_element(self) -> AnyElement {
        text(self).into_any_element()
    }
}

pub trait ParentElement: Sized {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>);

    fn child(mut self, child: impl IntoElement) -> Self {
        self.extend([child.into_any_element()]);
        self
    }

    fn children(mut self, children: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.extend(children.into_iter().map(IntoElement::into_any_element));
        self
    }
}

/// Props of a `div`. Backends map the visual part (background, radius, border, shadow,
/// opacity); layout fields are Taffy's and arrive as frames.
#[derive(Clone, Debug, PartialEq)]
pub struct DivProps {
    pub style: StyleRefinement,
}

/// Props of the window's content root.
#[derive(Clone, Debug, PartialEq)]
pub struct RootProps;

pub struct Div {
    key: Key,
    style: StyleRefinement,
    children: Vec<AnyElement>,
}

#[track_caller]
pub fn div() -> Div {
    Div { key: Key::Site(Location::caller()), style: StyleRefinement::default(), children: Vec::new() }
}

/// A row: flex, items centered.
#[track_caller]
pub fn h_flex() -> Div {
    let site = Location::caller();
    Div { key: Key::Site(site), style: StyleRefinement::default(), children: Vec::new() }
        .flex()
        .flex_row()
        .items_center()
}

/// A column.
#[track_caller]
pub fn v_flex() -> Div {
    let site = Location::caller();
    Div { key: Key::Site(site), style: StyleRefinement::default(), children: Vec::new() }.flex().flex_col()
}

impl Div {
    /// Keys the element explicitly, for siblings whose order or presence changes.
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        self.key = Key::Id(id.into());
        self
    }
}

impl Styled for Div {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Div {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl FluentBuilder for Div {}

impl IntoElement for Div {
    fn into_any_element(self) -> AnyElement {
        AnyElement::node(NodeElement {
            kind: NodeKind::View,
            key: self.key,
            props: Rc::new(DivProps { style: self.style.clone() }),
            layout: self.style,
            measured: false,
            children: self.children,
            handlers: Vec::new(),
        })
    }
}

/// Props of a text run.
#[derive(Clone, Debug, PartialEq)]
pub struct TextProps {
    pub text: SharedString,
    pub style: StyleRefinement,
    pub tabular_nums: bool,
}

pub struct Text {
    key: Key,
    props: TextProps,
}

#[track_caller]
pub fn text(text: impl Into<SharedString>) -> Text {
    Text {
        key: Key::Site(Location::caller()),
        props: TextProps { text: text.into(), style: StyleRefinement::default(), tabular_nums: false },
    }
}

impl Text {
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        self.key = Key::Id(id.into());
        self
    }

    /// Tabular figures, so changing digits keep their width.
    pub fn tabular_nums(mut self) -> Self {
        self.props.tabular_nums = true;
        self
    }
}

impl Styled for Text {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.props.style
    }
}

impl FluentBuilder for Text {}

impl IntoElement for Text {
    fn into_any_element(self) -> AnyElement {
        AnyElement::node(NodeElement {
            kind: NodeKind::Text,
            key: self.key,
            layout: self.props.style.clone(),
            props: Rc::new(self.props),
            measured: true,
            children: Vec::new(),
            handlers: Vec::new(),
        })
    }
}

/// A native widget leaf: `W` names the widget kind the backend maps, `P` its props. The
/// backend measures it and reports its events through the event queue.
pub struct NativeElement<P> {
    kind: WidgetKind,
    key: Key,
    props: P,
    style: StyleRefinement,
    handlers: Vec<(TypeId, Handler)>,
}

impl<P: AnyProps> NativeElement<P> {
    pub fn new<W: 'static>(key: impl Into<Key>, props: P) -> Self {
        Self {
            kind: WidgetKind::of::<W>(),
            key: key.into(),
            props,
            style: StyleRefinement::default(),
            handlers: Vec::new(),
        }
    }

    pub fn props_mut(&mut self) -> &mut P {
        &mut self.props
    }

    /// Handles events of type `E` the widget emits. Replaces an earlier handler for `E`.
    pub fn on<E: 'static>(mut self, handler: impl Fn(&E, &mut Window, &mut App) + 'static) -> Self {
        let handler: Handler = Rc::new(move |event, window, cx| {
            if let Some(event) = event.downcast_ref::<E>() {
                handler(event, window, cx);
            }
        });
        self.handlers.retain(|(t, _)| *t != TypeId::of::<E>());
        self.handlers.push((TypeId::of::<E>(), handler));
        self
    }
}

impl<P> Styled for NativeElement<P> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<P> FluentBuilder for NativeElement<P> {}

impl<P: AnyProps> IntoElement for NativeElement<P> {
    fn into_any_element(self) -> AnyElement {
        AnyElement::node(NodeElement {
            kind: NodeKind::Widget(self.kind),
            key: self.key,
            props: Rc::new(self.props),
            layout: self.style,
            measured: true,
            children: Vec::new(),
            handlers: self.handlers,
        })
    }
}

/// Fired by a button on activation (click, Space, Return as default button).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClickEvent;

#[derive(Clone, Debug, PartialEq)]
pub struct ButtonProps {
    pub label: SharedString,
}

/// A push button; `NSButton` on AppKit. Its widget kind is `WidgetKind::of::<Button>()`.
pub struct Button(NativeElement<ButtonProps>);

pub fn button(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Button {
    Button(NativeElement::new::<Button>(Key::Id(id.into()), ButtonProps { label: label.into() }))
}

impl Button {
    pub fn on_click(self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        Button(self.0.on(handler))
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl FluentBuilder for Button {}

impl IntoElement for Button {
    fn into_any_element(self) -> AnyElement {
        self.0.into_any_element()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowControlsProps {
    /// Pins the OS window controls with their first button's top-left at this point, in
    /// window coordinates; `None` leaves them where the OS puts them.
    pub at: Option<nd_core::Point>,
}

/// The window's own controls (traffic lights on macOS, ARCHITECTURE.md 5.4). The buttons stay
/// the OS's; the element only says where they go and takes no space in layout. Its widget kind
/// is `WidgetKind::of::<WindowControls>()`.
pub struct WindowControls(NativeElement<WindowControlsProps>);

#[track_caller]
pub fn window_controls() -> WindowControls {
    let element = NativeElement::new::<WindowControls>(Key::Site(Location::caller()), WindowControlsProps { at: None });
    WindowControls(element.absolute())
}

impl WindowControls {
    pub fn at(mut self, at: nd_gpui::gpui::Point<nd_gpui::gpui::Pixels>) -> Self {
        self.0.props_mut().at = Some(nd_core::Point { x: at.x.as_f32(), y: at.y.as_f32() });
        self
    }
}

impl IntoElement for WindowControls {
    fn into_any_element(self) -> AnyElement {
        self.0.into_any_element()
    }
}
