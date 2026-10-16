//! Small app-facing helpers: the image cache handle, drag-move events and `cx.processor`.
//! `canvas` and its `Painter` are `nd-ui`'s.

use nd_gpui::gpui::Point;
use nd_ui::prelude::*;

pub use nd_components::media::ImageSource;
pub use nd_ui::{Canvas, CanvasPath, Painter, canvas};

/// Images the app loads by URL or path, cached per app (`nd-system`, M7).
pub trait ImageCache {
    fn image(&mut self, source: &str) -> ImageSource;
}

impl ImageCache for App {
    fn image(&mut self, source: &str) -> ImageSource {
        ImageSource::from(source)
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
