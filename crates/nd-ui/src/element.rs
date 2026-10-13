//! Element descriptions: what `render` returns. Each becomes a retained node (or a view
//! boundary) through the reconciler; nothing here touches a backend.
//!
//! Identity (3.14): an element with an explicit id is keyed by it; every other element by the
//! `#[track_caller]` location of its constructor, plus an occurrence index among siblings with
//! the same location. `.when(..)` therefore never shifts a sibling onto another native node.

use std::any::{Any, TypeId};
use std::panic::Location;
use std::rc::Rc;

use nd_core::{NodeId, WindowId};
use nd_gpui::gpui::prelude::FluentBuilder;
use nd_gpui::gpui::{Action, KeyContext, SharedString, StyleRefinement, Styled};

use crate::app::{AnyView, App, EntityId};
use crate::backend::{AnyProps, NodeKind, Props, WidgetKind};
use crate::dispatch::{DispatchTree, Focusable};
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

/// Registers one `on_action` listener on the node's entry in its window's dispatch tree.
pub(crate) type ActionRegistrar = Rc<dyn Fn(&mut DispatchTree<App>, NodeId, WindowId)>;

/// What an element contributes to dispatch (ARCHITECTURE.md 3.12): its key context, action
/// listeners and focusability. The retained tree hands them to the window's [`DispatchTree`]
/// on every render.
#[derive(Clone, Default)]
pub struct DispatchProps {
    pub(crate) key_context: Option<KeyContext>,
    pub(crate) actions: Vec<ActionRegistrar>,
    pub(crate) focusable: Option<Focusable>,
}

/// Elements that take part in key dispatch: GPUI's `key_context` and `on_action`.
pub trait InteractiveElement: Sized {
    fn dispatch_props(&mut self) -> &mut DispatchProps;

    /// Names the context bindings match against while focus is in this element's subtree.
    /// Panics on a context that does not parse, like a malformed binding would.
    fn key_context<C, E>(mut self, context: C) -> Self
    where
        C: TryInto<KeyContext, Error = E>,
        E: std::fmt::Debug,
    {
        let context = context.try_into().unwrap_or_else(|e| panic!("invalid key context: {e:?}"));
        self.dispatch_props().key_context = Some(context);
        self
    }

    /// Makes the element a focus target: Tab reaches it in tree order, and bindings match in
    /// its contexts while it has focus.
    fn focusable(mut self) -> Self {
        self.dispatch_props().focusable.get_or_insert_with(Focusable::default);
        self
    }

    /// Focusable, ordered by `index` before tree order in Tab traversal (lower first).
    fn tab_index(mut self, index: isize) -> Self {
        self.dispatch_props().focusable.get_or_insert_with(Focusable::default).tab_index = index;
        self
    }

    /// Focusable; `false` keeps it out of Tab traversal while `focus` can still reach it.
    fn tab_stop(mut self, tab_stop: bool) -> Self {
        self.dispatch_props().focusable.get_or_insert_with(Focusable::default).tab_stop = tab_stop;
        self
    }

    /// Handles `A` when it bubbles through this element from the focused node. The action stops
    /// here unless the listener calls [`App::propagate`].
    fn on_action<A: Action>(mut self, listener: impl Fn(&A, &mut Window, &mut App) + 'static) -> Self {
        let listener = Rc::new(listener);
        let registrar: ActionRegistrar = Rc::new(move |tree, node, window_id| {
            let listener = listener.clone();
            tree.on_action(node, move |action: &A, acx| {
                let app = &mut *acx.cx;
                let Some(mut window) = app.windows.get_mut(&window_id).and_then(|w| w.window.take()) else {
                    acx.propagate();
                    return;
                };
                app.propagate_action = false;
                listener(action, &mut window, app);
                if let Some(slot) = app.windows.get_mut(&window_id) {
                    slot.window = Some(window);
                }
                if std::mem::take(&mut app.propagate_action) {
                    acx.propagate();
                }
            });
        });
        self.dispatch_props().actions.push(registrar);
        self
    }
}

pub(crate) struct NodeElement {
    pub kind: NodeKind,
    pub key: Key,
    pub props: Props,
    pub layout: StyleRefinement,
    /// The backend measures it (text, native widgets); containers are sized by Taffy alone.
    pub measured: bool,
    pub children: Vec<AnyElement>,
    pub handlers: Vec<(TypeId, Handler)>,
    pub dispatch: DispatchProps,
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
    /// Pointer-down moves the window (ARCHITECTURE.md 5.4). Interactive children opt out by
    /// hit-test: a button inside never starts a move.
    pub drag_region: bool,
    /// Padded by the window's [`TitlebarInsets`](crate::TitlebarInsets), so content never sits
    /// under the OS window controls.
    pub titlebar_insets: bool,
}

/// Props of the window's content root.
#[derive(Clone, Debug, PartialEq)]
pub struct RootProps;

pub struct Div {
    key: Key,
    style: StyleRefinement,
    children: Vec<AnyElement>,
    dispatch: DispatchProps,
    drag_region: bool,
    titlebar_insets: bool,
}

#[track_caller]
pub fn div() -> Div {
    Div::at(Location::caller())
}

/// A row: flex, items centered.
#[track_caller]
pub fn h_flex() -> Div {
    Div::at(Location::caller()).flex().flex_row().items_center()
}

/// A column.
#[track_caller]
pub fn v_flex() -> Div {
    Div::at(Location::caller()).flex().flex_col()
}

/// A container whose background moves the window: pointer-down starts the OS move, a double
/// click follows the platform (zoom on macOS, maximize elsewhere). Buttons and other interactive
/// children inside it keep their clicks.
#[track_caller]
pub fn drag_region() -> Div {
    let mut div = Div::at(Location::caller());
    div.drag_region = true;
    div
}

impl Div {
    fn at(site: &'static Location<'static>) -> Self {
        Div {
            key: Key::Site(site),
            style: StyleRefinement::default(),
            children: Vec::new(),
            dispatch: DispatchProps::default(),
            drag_region: false,
            titlebar_insets: false,
        }
    }

    /// Pads the element by the window's titlebar insets: the leading and trailing space the OS
    /// window controls take, and their height as a minimum. A custom title row uses it so no
    /// margin past the controls is tuned by hand.
    pub fn titlebar_insets(mut self) -> Self {
        self.titlebar_insets = true;
        self
    }

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

impl InteractiveElement for Div {
    fn dispatch_props(&mut self) -> &mut DispatchProps {
        &mut self.dispatch
    }
}

impl IntoElement for Div {
    fn into_any_element(self) -> AnyElement {
        AnyElement::node(NodeElement {
            kind: NodeKind::View,
            key: self.key,
            props: Rc::new(DivProps {
                style: self.style.clone(),
                drag_region: self.drag_region,
                titlebar_insets: self.titlebar_insets,
            }),
            layout: self.style,
            measured: false,
            children: self.children,
            handlers: Vec::new(),
            dispatch: self.dispatch,
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
            dispatch: DispatchProps::default(),
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
    dispatch: DispatchProps,
}

impl<P: AnyProps> NativeElement<P> {
    pub fn new<W: 'static>(key: impl Into<Key>, props: P) -> Self {
        Self {
            kind: WidgetKind::of::<W>(),
            key: key.into(),
            props,
            style: StyleRefinement::default(),
            handlers: Vec::new(),
            dispatch: DispatchProps::default(),
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

impl<P> InteractiveElement for NativeElement<P> {
    fn dispatch_props(&mut self) -> &mut DispatchProps {
        &mut self.dispatch
    }
}

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
            dispatch: self.dispatch,
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

impl InteractiveElement for Button {
    fn dispatch_props(&mut self) -> &mut DispatchProps {
        self.0.dispatch_props()
    }
}

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
