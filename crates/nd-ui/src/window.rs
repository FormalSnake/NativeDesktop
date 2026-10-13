use nd_core::{FrameTime, Size, WindowId};
use nd_gpui::gpui::{self, Pixels, SharedString};

use crate::app::Entity;

/// What to open. Window frames persist by `id` (5.3).
#[derive(Clone, Debug)]
pub struct WindowOptions {
    pub id: SharedString,
    pub title: Option<SharedString>,
    pub size: Option<gpui::Size<Pixels>>,
    pub min_size: Option<gpui::Size<Pixels>>,
}

impl WindowOptions {
    /// The app's main window.
    pub fn main(id: impl Into<SharedString>) -> Self {
        Self { id: id.into(), title: None, size: None, min_size: None }
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn size(mut self, size: gpui::Size<Pixels>) -> Self {
        self.size = Some(size);
        self
    }

    pub fn min_size(mut self, size: gpui::Size<Pixels>) -> Self {
        self.min_size = Some(size);
        self
    }
}

/// The room the OS window controls take in the window's top row, in logical points (5.4):
/// traffic lights leading on macOS, caption buttons trailing on Windows, either side under a
/// client-decorated Linux window, nothing under server decorations (Hyprland). The backend
/// reports them; [`Div::titlebar_insets`](crate::Div::titlebar_insets) pads by them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TitlebarInsets {
    pub leading: f32,
    pub trailing: f32,
    /// The height of the controls' row; a padded element is at least this tall.
    pub height: f32,
}

/// Per-window state handed to `render` and to handlers.
pub struct Window {
    id: WindowId,
    title: SharedString,
    viewport: Size,
    clock: FrameTime,
    insets: TitlebarInsets,
}

impl Window {
    pub(crate) fn new(id: WindowId, options: &WindowOptions) -> Self {
        let viewport = options
            .size
            .map(|s| Size { width: s.width.as_f32(), height: s.height.as_f32() })
            .unwrap_or(Size { width: 800., height: 600. });
        Self {
            id,
            title: options.title.clone().unwrap_or_default(),
            viewport,
            clock: FrameTime::default(),
            insets: TitlebarInsets::default(),
        }
    }

    pub fn id(&self) -> WindowId {
        self.id
    }

    pub fn title(&self) -> &SharedString {
        &self.title
    }

    /// Content size in logical points.
    pub fn viewport_size(&self) -> Size {
        self.viewport
    }

    pub(crate) fn set_viewport_size(&mut self, size: Size) {
        self.viewport = size;
    }

    /// Where the OS window controls sit, as the backend last reported them.
    pub fn titlebar_insets(&self) -> TitlebarInsets {
        self.insets
    }

    pub(crate) fn set_titlebar_insets(&mut self, insets: TitlebarInsets) {
        self.insets = insets;
    }

    /// The window's frame clock: its display's refresh on a real host, the test clock in tests.
    pub fn frame_time(&self) -> FrameTime {
        self.clock
    }

    /// When the frame being prepared will be presented, in seconds. Animations sample here.
    pub fn now(&self) -> f64 {
        self.clock.target
    }

    pub(crate) fn set_frame_time(&mut self, time: FrameTime) {
        self.clock = time;
    }
}

pub struct WindowHandle<V> {
    id: WindowId,
    root: Entity<V>,
}

impl<V: 'static> WindowHandle<V> {
    pub(crate) fn new(id: WindowId, root: Entity<V>) -> Self {
        Self { id, root }
    }

    pub fn id(&self) -> WindowId {
        self.id
    }

    pub fn root(&self) -> &Entity<V> {
        &self.root
    }
}

impl<V> Clone for WindowHandle<V> {
    fn clone(&self) -> Self {
        Self { id: self.id, root: self.root.clone() }
    }
}
