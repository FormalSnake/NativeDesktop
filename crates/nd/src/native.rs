//! App-defined native widgets (ARCHITECTURE.md 3.5): one trait, a drawn implementation for every
//! backend and optional AppKit or Swift ones. On AppKit a widget with a native implementation
//! mounts as a widget leaf the backend creates through its custom widget hook; everywhere else,
//! and on AppKit when the Swift symbols are missing, it mounts its drawn element tree under a
//! container that carries its semantics.

use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

pub use nd_core::semantic::Role;
pub use nd_ui::events::Synced;
use nd_ui::prelude::*;
use nd_ui::{Key, NativeElement as Leaf, Scope, StyleRefinement};

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

impl Semantics {
    fn to_tree(&self) -> nd_ui::semantics::Semantics {
        let name = self.label.as_ref().map(|l| l.to_string()).unwrap_or_default();
        let semantics = nd_ui::semantics::Semantics::new(self.role, name);
        match &self.value {
            Some(SemanticValue::Text(text)) => semantics.value(text.to_string()),
            Some(SemanticValue::Range { value, .. }) => semantics.value(value.to_string()),
            None => semantics,
        }
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
    sink: Option<Rc<dyn Fn(E)>>,
}

impl<E> Clone for Emit<E> {
    fn clone(&self) -> Self {
        Self { sink: self.sink.clone() }
    }
}

impl<E: 'static> Emit<E> {
    pub fn new(inner: nd_ui::events::Emit) -> Self {
        Self::from_fn(move |event| inner.emit(event))
    }

    pub fn detached() -> Self {
        Self { sink: None }
    }

    fn from_fn(sink: impl Fn(E) + 'static) -> Self {
        Self { sink: Some(Rc::new(sink)) }
    }

    pub fn emit(&self, event: E) {
        if let Some(sink) = &self.sink {
            sink(event);
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

/// Serializes a widget's props for its Swift view.
type ToJson<P> = fn(&P) -> String;

/// The native implementations a widget registers, per backend.
pub struct Backends<W: NativeWidget> {
    #[cfg(target_os = "macos")]
    appkit: Option<Rc<dyn appkit::AnyAppKitWidget<W>>>,
    swift: Option<(SwiftWidget, ToJson<W::Props>)>,
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

    /// A Swift or SwiftUI view from the app's Swift package (3.9), on macOS. The props cross to
    /// Swift as JSON.
    pub fn swift(mut self, widget: SwiftWidget) -> Self
    where
        W::Props: serde::Serialize,
    {
        fn json<P: serde::Serialize>(props: &P) -> String {
            serde_json::to_string(props).expect("a Swift widget's props serialize to JSON")
        }
        self.swift = Some((widget, json::<W::Props>));
        self
    }

    pub fn swift_widget(&self) -> Option<&SwiftWidget> {
        self.swift.as_ref().map(|(widget, _)| widget)
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

/// Names a Swift view exported by the app's Swift package. `SwiftWidget::named("Gauge")` is the
/// three `@_cdecl` functions `nd_widget_gauge_create`, `nd_widget_gauge_update` and
/// `nd_widget_gauge_measure`; a name of several words (`"MapView"`) joins them with underscores
/// (`nd_widget_map_view_create`).
#[derive(Clone, Debug, PartialEq)]
pub struct SwiftWidget {
    pub name: &'static str,
}

impl SwiftWidget {
    pub fn named(name: &'static str) -> Self {
        Self { name }
    }

    /// The C symbol of one of the view's functions (`"create"`, `"update"`, `"measure"`).
    pub fn symbol(&self, function: &str) -> String {
        let mut symbol = String::from("nd_widget_");
        for (i, c) in self.name.chars().enumerate() {
            if c.is_uppercase() && i > 0 {
                symbol.push('_');
            }
            symbol.extend(c.to_lowercase());
        }
        symbol.push('_');
        symbol.push_str(function);
        symbol
    }
}

type EventHandler<E> = Rc<dyn Fn(&E, &mut Window, &mut App)>;

/// What every native element carries besides its props: identity, layout style and handlers.
pub struct NativeBase<W: NativeWidget> {
    id: SharedString,
    style: StyleRefinement,
    handlers: Vec<EventHandler<W::Event>>,
}

impl<W: NativeWidget> NativeBase<W> {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into().0, style: StyleRefinement::default(), handlers: Vec::new() }
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

impl<W: NativeWidget> nd_ui::semantics::WidgetSemantics for WidgetProps<W> {
    fn semantics(&self) -> nd_ui::semantics::Semantics {
        W::semantics(&self.0).to_tree()
    }
}

/// What `#[derive(IntoElement)]` expands to: the widget's native leaf where the backend has a
/// native implementation of it, its drawn element tree everywhere else.
pub fn into_any_element<T: NativeElement>(element: T) -> AnyElement {
    let (base, props) = element.into_parts();
    deferred(move |window, cx| {
        #[cfg(target_os = "macos")]
        if cx.backend_as::<nd_backend_appkit::AppKitBackend>().is_some() && appkit::mounts::<T::Widget>() {
            return leaf(base, props);
        }
        drawn(base, props, window, cx)
    })
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn leaf<W: NativeWidget>(base: NativeBase<W>, props: W::Props) -> AnyElement {
    let mut leaf = Leaf::described::<W>(Key::Id(base.id), WidgetProps::<W>(props));
    *leaf.style() = base.style;
    let handlers = base.handlers;
    if !handlers.is_empty() {
        leaf = leaf.on(move |event: &W::Event, window, cx| {
            for handler in &handlers {
                handler(event, window, cx);
            }
        });
    }
    leaf.into_any_element()
}

/// The drawn implementation under a container that takes the element's id, style and semantics.
/// The drawn tree has no node of its own to address events to, so its emitter queues the call to
/// the element's handlers instead.
fn drawn<W: NativeWidget>(base: NativeBase<W>, props: W::Props, window: &mut Window, cx: &mut App) -> AnyElement {
    let emit = if base.handlers.is_empty() {
        Emit::detached()
    } else {
        let (queue, window_id, handlers) = (cx.events().clone(), window.id(), Rc::new(base.handlers));
        Emit::from_fn(move |event: W::Event| {
            let handlers = handlers.clone();
            queue.push_call(
                window_id,
                Box::new(move |window, cx| {
                    for handler in handlers.iter() {
                        handler(&event, window, cx);
                    }
                }),
            );
        })
    };
    let semantics = W::semantics(&props).to_tree();
    let mut root = div().id(base.id).scope(Scope::new(semantics)).child(W::drawn(&props, emit, window, cx));
    *root.style() = base.style;
    root.into_any_element()
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
    //! The AppKit side of a native widget: an NSView the widget creates, updates and measures,
    //! mounted through the backend's custom widget hook.

    use std::any::TypeId;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::{CString, c_char, c_void};
    use std::rc::Rc;

    use nd_backend_appkit::custom::{CustomCx, CustomNative, CustomWidget};
    pub use nd_backend_appkit::{objc2, objc2_app_kit, objc2_foundation};
    use nd_gpui::gpui::{Pixels, Size, px};
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, Message};
    use objc2_app_kit::NSView;

    use super::{Emit, NativeWidget, SwiftWidget, WidgetProps};

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
        events: nd_ui::events::Emit,
    }

    impl<W: NativeWidget> AppKitCx<W> {
        fn of(cx: &CustomCx<'_>) -> Self {
            let (events, waker) = (cx.emitter(), cx.waker());
            let queue = events.clone();
            // A delegate emits from the run loop, outside nd's dispatch, so the host must be woken
            // to drain the queue.
            let emit = Emit::from_fn(move |event: W::Event| {
                queue.emit(event);
                waker.wake();
            });
            Self { mtm: cx.mtm(), emit, events }
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
            self.events.programmatic(f)
        }
    }

    pub trait AppKitWidget<W: NativeWidget> {
        type View: Message + AsRef<NSView> + 'static;
        /// Per-node state the framework keeps beside the view: delegates, `Synced<T>` slots (3.13).
        type State: 'static;

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
    pub(super) trait AnyAppKitWidget<W: NativeWidget> {
        fn create_node(self: Rc<Self>, cx: &CustomCx<'_>) -> Box<dyn CustomNative>;
    }

    impl<W: NativeWidget, A: AppKitWidget<W> + 'static> AnyAppKitWidget<W> for A {
        fn create_node(self: Rc<Self>, cx: &CustomCx<'_>) -> Box<dyn CustomNative> {
            let props = props_of::<W>(cx).clone();
            let (view, state) = self.create(&props, &mut AppKitCx::of(cx));
            Box::new(AppKitNode { widget: self, view, state, props })
        }
    }

    fn props_of<'a, W: NativeWidget>(cx: &'a CustomCx<'_>) -> &'a W::Props {
        let props = cx.props().downcast_ref::<WidgetProps<W>>();
        &props.unwrap_or_else(|| panic!("{} node without its props", std::any::type_name::<W>())).0
    }

    fn to_size(size: Size<Pixels>) -> nd_core::Size {
        nd_core::Size { width: f32::from(size.width), height: f32::from(size.height) }
    }

    struct AppKitNode<W: NativeWidget, A: AppKitWidget<W>> {
        widget: Rc<A>,
        view: Retained<A::View>,
        state: A::State,
        props: W::Props,
    }

    impl<W: NativeWidget, A: AppKitWidget<W>> CustomNative for AppKitNode<W, A> {
        fn view(&self) -> &NSView {
            (*self.view).as_ref()
        }

        fn update(&mut self, cx: &CustomCx<'_>) {
            let new = props_of::<W>(cx);
            if *new != self.props {
                self.widget.update(&self.view, &mut self.state, &self.props, new, &mut AppKitCx::of(cx));
                self.props = new.clone();
            }
        }

        fn set_theme(&mut self, _theme: &nd_theme::Theme) {}

        fn measure(&self, width: Option<f32>) -> nd_core::Size {
            let available = AvailableSize { width: width.map(px), height: None };
            to_size(self.widget.measure(&self.view, &self.props, available))
        }
    }

    struct Registered<W: NativeWidget>(Rc<dyn AnyAppKitWidget<W>>);

    impl<W: NativeWidget> CustomWidget for Registered<W> {
        fn create(&self, cx: &CustomCx<'_>) -> Box<dyn CustomNative> {
            self.0.clone().create_node(cx)
        }
    }

    type SwiftCreate = unsafe extern "C" fn(*const c_char) -> *mut c_void;
    type SwiftUpdate = unsafe extern "C" fn(*mut c_void, *const c_char);
    type SwiftMeasure = unsafe extern "C" fn(*mut c_void, *mut f64, *mut f64);

    /// A Swift widget's `@_cdecl` functions (3.9): `create` returns a +1 retained view built from
    /// JSON props, `update` applies new JSON props, `measure` writes the fitting size.
    #[derive(Clone, Copy)]
    struct SwiftFns {
        create: SwiftCreate,
        update: SwiftUpdate,
        measure: SwiftMeasure,
    }

    impl SwiftFns {
        fn resolve(widget: &SwiftWidget) -> Option<Self> {
            let symbol = |function| nd_backend_appkit::swift_symbol(&widget.symbol(function));
            let (create, update, measure) = (symbol("create")?, symbol("update")?, symbol("measure")?);
            // SAFETY: the app's Swift exports these names with these signatures (3.9).
            unsafe {
                Some(Self {
                    create: std::mem::transmute::<*mut c_void, SwiftCreate>(create),
                    update: std::mem::transmute::<*mut c_void, SwiftUpdate>(update),
                    measure: std::mem::transmute::<*mut c_void, SwiftMeasure>(measure),
                })
            }
        }
    }

    fn c_json(json: &str) -> CString {
        CString::new(json).expect("JSON escapes NUL")
    }

    struct SwiftRegistered<W: NativeWidget> {
        fns: SwiftFns,
        json: super::ToJson<W::Props>,
    }

    impl<W: NativeWidget> CustomWidget for SwiftRegistered<W> {
        fn create(&self, cx: &CustomCx<'_>) -> Box<dyn CustomNative> {
            let json = (self.json)(props_of::<W>(cx));
            // SAFETY: called on the main thread, as the Swift side's `@MainActor` requires.
            let raw = unsafe { (self.fns.create)(c_json(&json).as_ptr()) };
            let view = unsafe { Retained::from_raw(raw.cast::<NSView>()) }
                .unwrap_or_else(|| panic!("{} returned no view", std::any::type_name::<W>()));
            Box::new(SwiftNode::<W> { view, fns: self.fns, to_json: self.json, json })
        }
    }

    struct SwiftNode<W: NativeWidget> {
        view: Retained<NSView>,
        fns: SwiftFns,
        to_json: super::ToJson<W::Props>,
        json: String,
    }

    impl<W: NativeWidget> SwiftNode<W> {
        fn raw(&self) -> *mut c_void {
            Retained::as_ptr(&self.view).cast_mut().cast()
        }
    }

    impl<W: NativeWidget> CustomNative for SwiftNode<W> {
        fn view(&self) -> &NSView {
            &self.view
        }

        fn update(&mut self, cx: &CustomCx<'_>) {
            let json = (self.to_json)(props_of::<W>(cx));
            if json != self.json {
                unsafe { (self.fns.update)(self.raw(), c_json(&json).as_ptr()) };
                self.json = json;
            }
        }

        fn set_theme(&mut self, _theme: &nd_theme::Theme) {}

        fn measure(&self, _width: Option<f32>) -> nd_core::Size {
            let (mut width, mut height) = (0.0, 0.0);
            unsafe { (self.fns.measure)(self.raw(), &mut width, &mut height) };
            nd_core::Size { width: width as f32, height: height as f32 }
        }
    }

    /// Whether `W` mounts as a native leaf on AppKit, registering its implementation with the
    /// backend the first time.
    pub(super) fn mounts<W: NativeWidget>() -> bool {
        thread_local!(static MOUNTS: RefCell<HashMap<TypeId, bool>> = RefCell::default());
        if let Some(mounts) = MOUNTS.with(|m| m.borrow().get(&TypeId::of::<W>()).copied()) {
            return mounts;
        }
        let backends = W::backends();
        let mounts = if let Some(widget) = backends.appkit {
            nd_backend_appkit::custom::register::<W>(Registered(widget));
            true
        } else if let Some((swift, json)) = backends.swift {
            match SwiftFns::resolve(&swift) {
                Some(fns) => {
                    nd_backend_appkit::custom::register::<W>(SwiftRegistered::<W> { fns, json });
                    true
                }
                None => {
                    eprintln!(
                        "nd: no `{}` in this binary (is the Swift package in `[macos] swift_packages` and \
                         `nd::build::app()` in build.rs?); {} is drawn",
                        swift.symbol("create"),
                        std::any::type_name::<W>()
                    );
                    false
                }
            }
        } else {
            false
        };
        MOUNTS.with(|m| m.borrow_mut().insert(TypeId::of::<W>(), mounts));
        mounts
    }
}

#[cfg(test)]
mod tests {
    use nd_ui::semantics::SemanticAction;
    use nd_ui::test_app::TestApp;
    use nd_ui::{Context, Render, Size};

    use super::*;

    /// A widget with only a drawn implementation: a button that turns the value up by one.
    struct Dial;

    #[derive(Clone, Debug, PartialEq)]
    struct Turned(f64);

    impl NativeWidget for Dial {
        type Props = f64;
        type Event = Turned;

        fn semantics(value: &f64) -> Semantics {
            Semantics::new(Role::Slider).label("Dial").value(SemanticValue::range(*value, 0., 10.))
        }

        fn drawn(value: &f64, emit: Emit<Turned>, _window: &mut Window, _cx: &mut App) -> AnyElement {
            let next = value + 1.;
            button("turn", "Turn").on_click(move |_, _, _| emit.emit(Turned(next))).into_any_element()
        }

        fn backends() -> Backends<Self> {
            Backends::drawn_only()
        }
    }

    struct DialElement(NativeBase<Dial>, f64);

    impl NativeElement for DialElement {
        type Widget = Dial;

        fn base_mut(&mut self) -> &mut NativeBase<Dial> {
            &mut self.0
        }

        fn into_parts(self) -> (NativeBase<Dial>, f64) {
            (self.0, self.1)
        }
    }

    impl IntoElement for DialElement {
        fn into_any_element(self) -> AnyElement {
            into_any_element(self)
        }
    }

    struct Host {
        value: f64,
    }

    impl Render for Host {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let mut dial = DialElement(NativeBase::new("dial"), self.value);
            dial.base_mut().on_event(cx.listener(|this, e: &Turned, _, cx| {
                this.value = e.0;
                cx.notify();
            }));
            v_flex().child(dial)
        }
    }

    #[test]
    fn a_drawn_only_widget_mounts_with_its_semantics_and_delivers_its_events() {
        let mut cx = TestApp::new();
        cx.open_window(Size { width: 200., height: 100. }, |_, cx| cx.new(|_| Host { value: 1. }));
        let find = |cx: &TestApp, id: &str| {
            cx.semantic_tree().nodes().find(|(_, n)| n.test_id.as_deref() == Some(id)).map(|(_, n)| n.clone())
        };
        let dial = find(&cx, "dial").expect("the dial mounts");
        assert_eq!((dial.role, dial.name.as_str(), dial.value.as_deref()), (Role::Slider, "Dial", Some("1")));

        let turn = find(&cx, "turn").expect("its drawn button mounts").id;
        cx.perform(turn, &SemanticAction::Click).unwrap();
        cx.run_until_idle();
        assert_eq!(find(&cx, "dial").unwrap().value.as_deref(), Some("2"), "the handler saw `Turned`");
    }
}
