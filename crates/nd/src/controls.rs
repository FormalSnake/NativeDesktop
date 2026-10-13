//! Stand-ins for what later milestones ship: source lists (M5 virtualization), canvases and
//! painting (M6), gestures (M4). They keep call sites compiling; the ones with no `nd-ui`
//! counterpart mount as an empty box with their id.

use std::marker::PhantomData;
use std::rc::Rc;

use nd_components::Icon;
use nd_gpui::gpui::{Bounds, Point};
use nd_ui::StyleRefinement;
use nd_ui::prelude::*;

type Handler<E> = Rc<dyn Fn(&E, &mut Window, &mut App)>;

/// An empty box keyed by the control's id: where a stand-in control mounts.
fn placeholder(id: &SharedString, style: StyleRefinement) -> AnyElement {
    let mut el = div().id(id.clone());
    *el.style() = style;
    el.into_any_element()
}

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
