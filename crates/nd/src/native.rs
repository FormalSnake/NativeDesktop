//! App-defined native widgets (ARCHITECTURE.md 3.5): one trait, a drawn implementation for every
//! backend and optional AppKit or Swift ones. M0 state: the call-site shape. An element mounts as
//! a widget node carrying its props; backends map no app widget until M6, so nothing here draws
//! yet.

use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

pub use nd_core::semantic::Role;
pub use nd_ui::events::Synced;
use nd_ui::prelude::*;
use nd_ui::{Key, NativeElement as Leaf, StyleRefinement};

/// What a widget is to assistive technology and to automation.
#[derive(Clone, Debug, PartialEq)]
pub struct Semantics {
    pub role: Role,
    pub label: Option<SharedString>,
    pub value: Option<SemanticValue>,
}

impl Semantics {
    pub fn new(role: Role) -> Self {
        Self { role, label: None, value: None }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn value(mut self, value: impl Into<SemanticValue>) -> Self {
        self.value = Some(value.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SemanticValue {
    Text(SharedString),
    Range { value: f64, min: f64, max: f64 },
}

impl SemanticValue {
    pub fn range(value: f64, min: f64, max: f64) -> Self {
        SemanticValue::Range { value, min, max }
    }
}

impl From<String> for SemanticValue {
    fn from(text: String) -> Self {
        SemanticValue::Text(text.into())
    }
}

impl From<&'static str> for SemanticValue {
    fn from(text: &'static str) -> Self {
        SemanticValue::Text(text.into())
    }
}

/// Emits a widget's events into the window's event queue (3.13); never calls app code. A widget
/// built outside a window (a test, the stand-in) emits into nothing.
pub struct Emit<E> {
    inner: Option<nd_ui::events::Emit>,
    _event: PhantomData<fn(E)>,
}

impl<E> Clone for Emit<E> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), _event: PhantomData }
    }
}

impl<E: 'static> Emit<E> {
    pub fn new(inner: nd_ui::events::Emit) -> Self {
        Self { inner: Some(inner), _event: PhantomData }
    }

    pub fn detached() -> Self {
        Self { inner: None, _event: PhantomData }
    }

    pub fn emit(&self, event: E) {
        if let Some(inner) = &self.inner {
            inner.emit(event);
        }
    }
}

/// A widget: its props, its events, its meaning and how each backend shows it.
pub trait NativeWidget: 'static + Sized {
    type Props: Clone + PartialEq + 'static;
    type Event: 'static;

    fn semantics(props: &Self::Props) -> Semantics;

    /// The framework-drawn implementation, used wherever no native one is registered.
    fn drawn(props: &Self::Props, emit: Emit<Self::Event>, window: &mut Window, cx: &mut App) -> AnyElement;

    fn backends() -> Backends<Self>;
}

/// The native implementations a widget registers, per backend.
pub struct Backends<W: NativeWidget> {
    #[cfg(target_os = "macos")]
    appkit: Option<Rc<dyn appkit::AnyAppKitWidget<W>>>,
    swift: Option<SwiftWidget>,
    _widget: PhantomData<fn() -> W>,
}

impl<W: NativeWidget> Backends<W> {
    pub fn drawn_only() -> Self {
        Self {
            #[cfg(target_os = "macos")]
            appkit: None,
            swift: None,
            _widget: PhantomData,
        }
    }

    /// A Swift or SwiftUI view from the app's Swift package (3.9), on macOS.
    pub fn swift(mut self, widget: SwiftWidget) -> Self {
        self.swift = Some(widget);
        self
    }

    pub fn swift_widget(&self) -> Option<&SwiftWidget> {
        self.swift.as_ref()
    }

    #[cfg(target_os = "macos")]
    pub fn appkit(mut self, widget: impl appkit::AppKitWidget<W> + 'static) -> Self {
        self.appkit = Some(Rc::new(widget));
        self
    }

    #[cfg(target_os = "macos")]
    pub fn has_appkit(&self) -> bool {
        self.appkit.is_some()
    }
}

/// Names a Swift view exported by the app's Swift package.
#[derive(Clone, Debug, PartialEq)]
pub struct SwiftWidget {
    pub name: &'static str,
}

impl SwiftWidget {
    pub fn named(name: &'static str) -> Self {
        Self { name }
    }
}

type EventHandler<E> = Rc<dyn Fn(&E, &mut Window, &mut App)>;

/// What every native element carries besides its props: identity, layout style and handlers.
pub struct NativeBase<W: NativeWidget> {
    key: Key,
    style: StyleRefinement,
    handlers: Vec<EventHandler<W::Event>>,
}

impl<W: NativeWidget> NativeBase<W> {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { key: Key::Id(id.into().0), style: StyleRefinement::default(), handlers: Vec::new() }
    }

    /// Runs `handler` for every event the widget emits.
    pub fn on_event(&mut self, handler: impl Fn(&W::Event, &mut Window, &mut App) + 'static) {
        self.handlers.push(Rc::new(handler));
    }
}

/// A stateful element's explicit id (3.14).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ElementId(pub SharedString);

impl From<&'static str> for ElementId {
    fn from(id: &'static str) -> Self {
        ElementId(id.into())
    }
}

impl From<SharedString> for ElementId {
    fn from(id: SharedString) -> Self {
        ElementId(id)
    }
}

impl From<String> for ElementId {
    fn from(id: String) -> Self {
        ElementId(id.into())
    }
}

/// An element backed by a [`NativeWidget`]. `#[derive(IntoElement)]` mounts it.
pub trait NativeElement: Sized {
    type Widget: NativeWidget;

    fn base_mut(&mut self) -> &mut NativeBase<Self::Widget>;

    fn into_parts(self) -> (NativeBase<Self::Widget>, <Self::Widget as NativeWidget>::Props);
}

/// The props as the retained tree holds them.
pub struct WidgetProps<W: NativeWidget>(pub W::Props);

impl<W: NativeWidget> fmt::Debug for WidgetProps<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WidgetProps<{}>", std::any::type_name::<W>())
    }
}

impl<W: NativeWidget> PartialEq for WidgetProps<W> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

/// What `#[derive(IntoElement)]` expands to: a widget leaf with the element's props and handlers.
pub fn into_any_element<T: NativeElement>(element: T) -> AnyElement {
    let (base, props) = element.into_parts();
    let mut leaf = Leaf::new::<T::Widget>(base.key, WidgetProps::<T::Widget>(props));
    *leaf.style() = base.style;
    let handlers = base.handlers;
    if !handlers.is_empty() {
        leaf = leaf.on(move |event: &<T::Widget as NativeWidget>::Event, window, cx| {
            for handler in &handlers {
                handler(event, window, cx);
            }
        });
    }
    leaf.into_any_element()
}

impl<W: NativeWidget> Styled for NativeBase<W> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<T: NativeElement> NativeStyled for T {}

/// Layout styling on any native element, through its base. `Styled` itself cannot be implemented
/// for every `NativeElement` from here (both are foreign to the app crate), so the common
/// shorthands live on this trait.
pub trait NativeStyled: NativeElement {
    fn size_full(mut self) -> Self {
        let style = self.base_mut().style();
        style.size.width = Some(relative(1.).into());
        style.size.height = Some(relative(1.).into());
        self
    }
}

#[cfg(target_os = "macos")]
pub mod appkit {
    //! The AppKit side of a native widget: an NSView the widget creates, updates and measures.

    use std::marker::PhantomData;

    pub use nd_backend_appkit::{objc2, objc2_app_kit, objc2_foundation};
    use nd_gpui::gpui::{Pixels, Size};
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, Message};

    use super::{Emit, NativeWidget};

    /// The space a container offers a measured view; `None` is unbounded.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct AvailableSize {
        pub width: Option<Pixels>,
        pub height: Option<Pixels>,
    }

    /// What a widget's AppKit side gets while it creates or updates its view.
    pub struct AppKitCx<W: NativeWidget> {
        mtm: MainThreadMarker,
        emit: Emit<W::Event>,
        events: Option<nd_ui::events::EventQueue>,
        _widget: PhantomData<fn() -> W>,
    }

    impl<W: NativeWidget> AppKitCx<W> {
        pub fn new(mtm: MainThreadMarker, emit: Emit<W::Event>, events: Option<nd_ui::events::EventQueue>) -> Self {
            Self { mtm, emit, events, _widget: PhantomData }
        }

        pub fn mtm(&self) -> MainThreadMarker {
            self.mtm
        }

        /// An emitter for delegates and targets the view installs.
        pub fn emitter(&self) -> Emit<W::Event> {
            self.emit.clone()
        }

        /// Runs a setter whose synchronous callbacks must not reach the app (3.13 rule 2).
        pub fn programmatic<R>(&self, f: impl FnOnce() -> R) -> R {
            match &self.events {
                Some(events) => events.programmatic(f),
                None => f(),
            }
        }
    }

    pub trait AppKitWidget<W: NativeWidget> {
        type View: Message;
        type State;

        fn create(&self, props: &W::Props, cx: &mut AppKitCx<W>) -> (Retained<Self::View>, Self::State);

        fn update(
            &self,
            view: &Self::View,
            state: &mut Self::State,
            old: &W::Props,
            new: &W::Props,
            cx: &mut AppKitCx<W>,
        );

        fn measure(&self, view: &Self::View, props: &W::Props, available: AvailableSize) -> Size<Pixels>;
    }

    /// An [`AppKitWidget`] with its view and state types erased, as [`super::Backends`] keeps it.
    pub(super) trait AnyAppKitWidget<W: NativeWidget> {}

    impl<W: NativeWidget, A: AppKitWidget<W>> AnyAppKitWidget<W> for A {}
}
