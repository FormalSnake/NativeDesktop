//! Stand-ins for controls `nd-components` ships in M6: icons, button styles, sliders, selects,
//! source lists and canvases, plus the per-platform and app-level override hooks. They keep call
//! sites compiling; the ones with no `nd-ui` counterpart mount as an empty box with their id.

use std::marker::PhantomData;
use std::ops::RangeInclusive;
use std::rc::Rc;

use nd_gpui::gpui::{Action, Bounds, Point};
use nd_ui::prelude::*;
use nd_ui::{StyleRefinement, WindowControls};

use crate::theme::ThemeOverride;

/// Lucide names; SF Symbols on macOS and Segoe Fluent on Windows map from them (`nd-icons`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Archive,
    CircleCheck,
    FileText,
    MapPin,
    Notebook,
    PanelLeft,
    Pin,
    Settings,
    Share,
    SquarePen,
    Trash,
}

/// Per-platform adjustments on any element; the closure runs only on that platform.
pub trait OnPlatform: Sized {
    fn on_macos(self, f: impl FnOnce(Self) -> Self) -> Self {
        if cfg!(target_os = "macos") { f(self) } else { self }
    }

    fn on_windows(self, f: impl FnOnce(Self) -> Self) -> Self {
        if cfg!(target_os = "windows") { f(self) } else { self }
    }

    fn on_linux(self, f: impl FnOnce(Self) -> Self) -> Self {
        if cfg!(target_os = "linux") { f(self) } else { self }
    }

    fn on_web(self, f: impl FnOnce(Self) -> Self) -> Self {
        if cfg!(target_family = "wasm") { f(self) } else { self }
    }
}

impl<T: IntoElement> OnPlatform for T {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Bezel {
    #[default]
    Automatic,
    Glass,
    Bordered,
    Borderless,
}

/// Button variants beyond the label. Visual ones are recorded until `nd-components`.
pub trait ButtonExt: Sized {
    fn icon(self, icon: Icon) -> Self;
    /// Clicking dispatches `action` from the focused node, as a menu item or key binding would.
    fn dispatch(self, action: impl Action) -> Self;
    fn bezel(self, bezel: Bezel) -> Self;
    fn primary(self) -> Self;
    fn accent(self) -> Self;
    fn outline(self) -> Self;
}

impl ButtonExt for Button {
    fn icon(self, _icon: Icon) -> Self {
        self
    }

    fn dispatch(self, action: impl Action) -> Self {
        self.on_click(move |_: &ClickEvent, window, cx| {
            cx.dispatch_action(window.id(), &*action.boxed_clone());
        })
    }

    fn bezel(self, _bezel: Bezel) -> Self {
        self
    }

    fn primary(self) -> Self {
        self
    }

    fn accent(self) -> Self {
        self
    }

    fn outline(self) -> Self {
        self
    }
}

pub trait WindowControlsExt {
    /// Puts the controls inside the toolbar's leading edge, as a unified toolbar does.
    fn embedded(self) -> Self;
}

impl WindowControlsExt for WindowControls {
    fn embedded(self) -> Self {
        self
    }
}

/// An empty box keyed by the control's id: where a stand-in control mounts.
fn placeholder(id: &SharedString, style: StyleRefinement) -> AnyElement {
    let mut el = div().id(id.clone());
    *el.style() = style;
    el.into_any_element()
}

macro_rules! styled_stand_in {
    ($ty:ty) => {
        impl Styled for $ty {
            fn style(&mut self) -> &mut StyleRefinement {
                &mut self.style
            }
        }

        impl FluentBuilder for $ty {}

        impl IntoElement for $ty {
            fn into_any_element(self) -> AnyElement {
                placeholder(&self.id, self.style)
            }
        }
    };
}

type Handler<E> = Rc<dyn Fn(&E, &mut Window, &mut App)>;

#[derive(Clone, Debug, PartialEq)]
pub struct SliderChanged {
    pub value: f32,
}

pub struct Slider {
    id: SharedString,
    style: StyleRefinement,
    pub value: f32,
    pub range: RangeInclusive<f32>,
    pub drawn: bool,
    on_change: Option<Handler<SliderChanged>>,
}

pub fn slider(id: impl Into<SharedString>, value: f32) -> Slider {
    Slider { id: id.into(), style: StyleRefinement::default(), value, range: 0.0..=1.0, drawn: false, on_change: None }
}

impl Slider {
    pub fn range(mut self, range: RangeInclusive<f32>) -> Self {
        self.range = range;
        self
    }

    /// The framework-drawn slider on every OS instead of the native control.
    pub fn drawn(mut self) -> Self {
        self.drawn = true;
        self
    }

    pub fn on_change(mut self, f: impl Fn(&SliderChanged, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

styled_stand_in!(Slider);

/// How a control is presented, where the platform has more than one control for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mapping {
    #[default]
    Automatic,
    PopUp,
    Segmented,
    Radio,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectChanged {
    pub index: usize,
}

pub struct Select {
    id: SharedString,
    style: StyleRefinement,
    pub options: Vec<SharedString>,
    pub selected: usize,
    pub mapping: Mapping,
    on_change: Option<Handler<SelectChanged>>,
}

pub fn select(id: impl Into<SharedString>, options: impl IntoIterator<Item = impl Into<SharedString>>) -> Select {
    Select {
        id: id.into(),
        style: StyleRefinement::default(),
        options: options.into_iter().map(Into::into).collect(),
        selected: 0,
        mapping: Mapping::default(),
        on_change: None,
    }
}

impl Select {
    pub fn selected(mut self, index: usize) -> Self {
        self.selected = index;
        self
    }

    pub fn native(mut self, mapping: Mapping) -> Self {
        self.mapping = mapping;
        self
    }

    pub fn on_change(mut self, f: impl Fn(&SelectChanged, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

styled_stand_in!(Select);

/// The rows a virtualized list shows, by key (`nd-virtual`, M5).
pub struct ListState<K> {
    items: Vec<K>,
}

impl<K: Clone> ListState<K> {
    pub fn new(_cx: &mut App) -> Self {
        Self { items: Vec::new() }
    }

    pub fn set_items(&mut self, items: Vec<K>, _cx: &mut App) {
        self.items = items;
    }

    pub fn items(&self) -> &[K] {
        &self.items
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectEvent<K> {
    pub key: K,
}

/// A sidebar source list: `NSOutlineView` with the source-list style on macOS.
pub struct SourceList<K> {
    id: SharedString,
    style: StyleRefinement,
    on_select: Option<Handler<SelectEvent<K>>>,
    _key: PhantomData<fn() -> K>,
}

pub fn source_list<K: Clone + 'static, R: IntoElement>(
    id: impl Into<SharedString>,
    _state: &ListState<K>,
    _row: impl Fn(K, &mut Window, &mut App) -> R + 'static,
) -> SourceList<K> {
    SourceList { id: id.into(), style: StyleRefinement::default(), on_select: None, _key: PhantomData }
}

impl<K> SourceList<K> {
    pub fn on_select(mut self, f: impl Fn(&SelectEvent<K>, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }
}

impl<K> Styled for SourceList<K> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<K> IntoElement for SourceList<K> {
    fn into_any_element(self) -> AnyElement {
        placeholder(&self.id, self.style)
    }
}

/// One source list row: an icon and a label.
#[track_caller]
pub fn source_row(_icon: Icon, label: impl Into<SharedString>) -> Div {
    h_flex().gap_2().px_2().py_1().child(text(label.into()))
}

/// Painting primitives for `canvas` (`Painter`, 3.1).
pub struct Painter {
    _private: (),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageSource(pub SharedString);

impl Painter {
    /// A stroked arc around `center`, from `start` to `end` as fractions of a turn from 12 o'clock.
    pub fn stroke_arc(
        &mut self,
        _center: Point<Pixels>,
        _radius: Pixels,
        _start: f32,
        _end: f32,
        _width: Pixels,
        _color: Hsla,
    ) {
    }

    pub fn fill_circle(&mut self, _center: Point<Pixels>, _radius: Pixels, _color: Hsla) {}

    pub fn image(&mut self, _image: ImageSource, _bounds: Bounds<Pixels>) {}
}

type Paint = Rc<dyn Fn(Bounds<Pixels>, &mut Painter, &mut App)>;

/// Custom drawing, called with the element's bounds on every paint.
pub struct Canvas {
    style: StyleRefinement,
    _paint: Paint,
}

pub fn canvas(paint: impl Fn(Bounds<Pixels>, &mut Painter, &mut App) + 'static) -> Canvas {
    Canvas { style: StyleRefinement::default(), _paint: Rc::new(paint) }
}

impl Styled for Canvas {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl IntoElement for Canvas {
    #[track_caller]
    fn into_any_element(self) -> AnyElement {
        let mut el = div();
        *el.style() = self.style;
        el.into_any_element()
    }
}

/// Images the app loads by URL or path, cached per app (`nd-system`, M7).
pub trait ImageCache {
    fn image(&mut self, source: &str) -> ImageSource;
}

impl ImageCache for App {
    fn image(&mut self, source: &str) -> ImageSource {
        ImageSource(SharedString::from(source.to_owned()))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DragMoveEvent {
    /// Movement since the previous drag event.
    pub delta: Point<Pixels>,
}

/// Pointer gestures on any container (`nd-ui` gestures, M4).
pub trait Gestures: Sized {
    fn on_drag_move(self, _f: impl Fn(&DragMoveEvent, &mut Window, &mut App) + 'static) -> Self {
        self
    }
}

impl Gestures for Div {}

/// GPUI's `cx.processor`: a closure that updates this entity with the extra argument.
pub trait Processor<T> {
    fn processor<E, R>(
        &self,
        f: impl Fn(&mut T, E, &mut Window, &mut Context<T>) -> R + 'static,
    ) -> impl Fn(E, &mut Window, &mut App) -> R + 'static;
}

impl<T: 'static> Processor<T> for Context<'_, T> {
    fn processor<E, R>(
        &self,
        f: impl Fn(&mut T, E, &mut Window, &mut Context<T>) -> R + 'static,
    ) -> impl Fn(E, &mut Window, &mut App) -> R + 'static {
        let this = self.weak_entity();
        move |event, window, cx| this.update(cx, |this, cx| f(this, event, window, cx)).expect("entity released")
    }
}

/// How the window's title bar is drawn (5.4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Titlebar {
    #[default]
    Native,
    /// The app draws it with `titlebar()`; the OS controls stay the OS's.
    Custom,
}

pub trait WindowOptionsExt {
    fn titlebar(self, titlebar: Titlebar) -> Self;
}

impl WindowOptionsExt for WindowOptions {
    fn titlebar(self, _titlebar: Titlebar) -> Self {
        self
    }
}

pub trait WindowThemeExt {
    /// Overrides the theme for this window only.
    fn set_theme(&mut self, theme: ThemeOverride, cx: &mut App);
}

impl WindowThemeExt for Window {
    fn set_theme(&mut self, _theme: ThemeOverride, _cx: &mut App) {}
}
