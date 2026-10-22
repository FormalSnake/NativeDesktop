//! What the overlay layer covers in each window this frame, for native surfaces that the display
//! server stacks above everything GPUI draws (a Chrome-style page's X11 child window). Such a
//! surface cuts its shape around what [`occluders`] reports and stops taking input while an
//! overlay blocks the window, so overlays stay above it and nothing leaks through.
//!
//! Overlays record themselves while they prepaint, which GPUI does for the deferred layer after
//! the rest of the window and before anything paints: a surface reads the current frame's list
//! from its own paint. A surface also says from its paint that it was drawn ([`shown`]), and is
//! told once a frame goes by without it, since an element that is no longer drawn hears nothing.

use std::cell::RefCell;
use std::collections::HashMap;

use nd_gpui::gpui::{
    self, AnyElement, App, Bounds, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels,
};

/// One overlay surface in window coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Occluder {
    pub bounds: Bounds<Pixels>,
    /// It draws there: a surface under it must not show through.
    pub draws: bool,
    /// It takes the pointer there (a backdrop that blocks the whole window takes it everywhere).
    pub blocks_input: bool,
}

type Hide = Box<dyn FnOnce()>;

thread_local! {
    static FRAME: RefCell<HashMap<gpui::WindowId, Vec<Occluder>>> = RefCell::default();
    /// Surfaces drawn this frame, and those drawn the frame before.
    static SHOWN: RefCell<HashMap<gpui::WindowId, HashMap<u64, Hide>>> = RefCell::default();
    static LIVE: RefCell<HashMap<gpui::WindowId, HashMap<u64, Hide>>> = RefCell::default();
    /// The overlay whose content is painting, as its index in the frame's list.
    static PAINTING: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// A window's frame starts: its overlays record themselves afresh, and once it is drawn the
/// surfaces it no longer drew hide.
pub(crate) fn begin_frame(window: &gpui::Window, cx: &mut App) {
    let id = window.window_handle().window_id();
    FRAME.with(|f| f.borrow_mut().remove(&id));
    SHOWN.with(|s| s.borrow_mut().remove(&id));
    cx.defer(move |_| end_frame(id));
}

fn end_frame(id: gpui::WindowId) {
    let shown = SHOWN.with(|s| s.borrow_mut().remove(&id)).unwrap_or_default();
    let mut before = LIVE.with(|l| l.borrow_mut().remove(&id)).unwrap_or_default();
    before.retain(|key, _| !shown.contains_key(key));
    if !shown.is_empty() {
        LIVE.with(|l| l.borrow_mut().insert(id, shown));
    }
    for (_, hide) in before {
        hide();
    }
}

/// A native surface `key` was drawn in `window` this frame; `hide` runs after the first frame
/// that does not draw it.
pub fn shown(window: &gpui::Window, key: u64, hide: impl FnOnce() + 'static) {
    let id = window.window_handle().window_id();
    SHOWN.with(|s| s.borrow_mut().entry(id).or_default().insert(key, Box::new(hide)));
}

/// What the overlay layer covers in `window` this frame, above what is painting: a surface
/// inside an overlay (a page in a popover) is covered only by the overlays over that one.
pub fn occluders(window: &gpui::Window) -> Vec<Occluder> {
    let above = PAINTING.with(|p| p.get()).map_or(0, |i| i + 1);
    FRAME.with(|f| {
        f.borrow()
            .get(&window.window_handle().window_id())
            .map(|o| o[above.min(o.len())..].to_vec())
            .unwrap_or_default()
    })
}

/// Records its child's bounds as an occluder of its window.
pub(crate) struct Occluding {
    inner: AnyElement,
    draws: bool,
    blocks_input: bool,
}

impl Occluding {
    /// Content that draws over what is under it and takes the pointer there.
    pub(crate) fn surface(inner: impl IntoElement) -> Self {
        Self { inner: inner.into_any_element(), draws: true, blocks_input: true }
    }

    /// A transparent backdrop that takes the pointer and draws nothing.
    pub(crate) fn backdrop(inner: impl IntoElement) -> Self {
        Self { inner: inner.into_any_element(), draws: false, blocks_input: true }
    }

    /// Content that draws and takes no input (a tooltip, a toast's shadow).
    pub(crate) fn passive(inner: impl IntoElement) -> Self {
        Self { inner: inner.into_any_element(), draws: true, blocks_input: false }
    }
}

impl IntoElement for Occluding {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl gpui::Element for Occluding {
    type RequestLayoutState = ();
    /// The occluder's index in the frame's list, which is the order overlays paint in.
    type PrepaintState = usize;

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
    ) -> usize {
        let occluder = Occluder { bounds, draws: self.draws, blocks_input: self.blocks_input };
        let index = FRAME.with(|f| {
            let mut f = f.borrow_mut();
            let list = f.entry(window.window_handle().window_id()).or_default();
            list.push(occluder);
            list.len() - 1
        });
        self.inner.prepaint(window, cx);
        index
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut (),
        index: &mut usize,
        window: &mut gpui::Window,
        cx: &mut App,
    ) {
        let outer = PAINTING.with(|p| p.replace(Some(*index)));
        self.inner.paint(window, cx);
        PAINTING.with(|p| p.set(outer));
    }
}
