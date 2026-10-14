//! Stand-ins for what later milestones ship: canvases and painting (M6), gestures (M4). They
//! keep call sites compiling.

use std::rc::Rc;

use nd_gpui::gpui::{Bounds, Point};
use nd_ui::StyleRefinement;
use nd_ui::prelude::*;

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
