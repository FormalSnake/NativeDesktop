use std::rc::Rc;

use nd_core::{FrameTime, Size, WindowId};
use nd_gpui::gpui::{self, Pixels, SharedString};

use crate::app::{App, Entity};

/// How the window's title bar is drawn (ARCHITECTURE.md 5.4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Titlebar {
    /// The OS title bar, or the toolbar a `scaffold()` puts in it.
    #[default]
    Native,
    /// Content runs under a transparent title bar; the app draws its own row with
    /// `titlebar()`. The OS window controls stay the OS's.
    Custom,
}

/// What kind of window this is, which decides how the platform presents it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WindowKind {
    #[default]
    Normal,
    /// A sheet attached to `parent`: a window-modal sheet on macOS, a dialog with a scrim
    /// elsewhere.
    Sheet { parent: WindowId },
    /// The app's settings window (`cmd-,` on macOS).
    Settings,
}

/// What to open. Window frames persist by `id` (5.3).
#[derive(Clone, Debug)]
pub struct WindowOptions {
    pub id: SharedString,
    pub title: Option<SharedString>,
    pub size: Option<gpui::Size<Pixels>>,
    pub min_size: Option<gpui::Size<Pixels>>,
    pub titlebar: Titlebar,
    pub kind: WindowKind,
    /// Windows naming the same group show as tabs of one window (ARCHITECTURE.md 5.5): native
    /// window tabs on macOS, a drawn tab bar elsewhere. Read when the window opens; groups with
    /// different names never merge.
    pub tab_group: Option<SharedString>,
    /// Opening it makes it key and activates the app. Off (`.background()`), it opens behind
    /// whatever has focus: a launch from a script or a test that must not steal it.
    pub activate: bool,
    /// Where the window's top-left corner opens, in global points from the primary screen's
    /// top-left. Centred when unset.
    pub origin: Option<nd_core::Point>,
}

impl WindowOptions {
    /// The app's main window.
    pub fn main(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: None,
            size: None,
            min_size: None,
            titlebar: Titlebar::Native,
            kind: WindowKind::Normal,
            tab_group: None,
            activate: true,
            origin: None,
        }
    }

    /// Opens with the window's top-left corner at `at` (global points), as a tab dragged out of
    /// a window opens where it was dropped.
    pub fn origin(mut self, at: nd_core::Point) -> Self {
        self.origin = Some(at);
        self
    }

    /// Opens without taking focus or activating the app.
    pub fn background(mut self) -> Self {
        self.activate = false;
        self
    }

    /// Opens the window as a tab of `group`, next to the group's key window when one is open.
    pub fn tab_group(mut self, group: impl Into<SharedString>) -> Self {
        self.tab_group = Some(group.into());
        self
    }

    pub fn titlebar(mut self, titlebar: Titlebar) -> Self {
        self.titlebar = titlebar;
        self
    }

    pub fn kind(mut self, kind: WindowKind) -> Self {
        self.kind = kind;
        self
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

pub(crate) type ShouldClose = Rc<dyn Fn(&mut Window, &mut App) -> bool>;
pub(crate) type NewTab = Rc<dyn Fn(&mut Window, &mut App)>;

/// Per-window state handed to `render` and to handlers.
pub struct Window {
    id: WindowId,
    title: SharedString,
    viewport: Size,
    clock: FrameTime,
    options: WindowOptions,
    insets: TitlebarInsets,
    /// An element id to focus once the frame has rendered; see [`Window::focus`].
    pub(crate) focus_request: Option<SharedString>,
    pub(crate) should_close: Option<ShouldClose>,
    pub(crate) new_tab: Option<NewTab>,
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
            options: options.clone(),
            insets: TitlebarInsets::default(),
            focus_request: None,
            should_close: None,
            new_tab: None,
        }
    }

    /// Decides what the user's close does (the close button, `cmd-w` on a native tab, a drawn
    /// tab's close). Returning false holds the close: the window stays until the app calls
    /// [`App::close_window`], which is how a tab whose view the app still has to drop waits.
    /// Without a handler a close request closes the window. Replaces an earlier handler.
    pub fn on_should_close(&mut self, handler: impl Fn(&mut Window, &mut App) -> bool + 'static) {
        self.should_close = Some(Rc::new(handler));
    }

    /// The user asked this window's tab group for a new tab (the `+` of the tab bar). The app
    /// opens another window with the same [`WindowOptions::tab_group`]. Replaces an earlier
    /// handler.
    pub fn on_new_tab(&mut self, handler: impl Fn(&mut Window, &mut App) + 'static) {
        self.new_tab = Some(Rc::new(handler));
    }

    /// Whether the window answers a new tab request, so a drawn tab bar shows its `+`.
    pub fn handles_new_tab(&self) -> bool {
        self.new_tab.is_some()
    }

    /// What the window was opened with.
    pub fn options(&self) -> &WindowOptions {
        &self.options
    }

    pub fn id(&self) -> WindowId {
        self.id
    }

    /// Focuses the element given `.id(id)` in this window; see [`App::focus_id`].
    pub fn focus_id(&self, cx: &mut App, id: &str) -> bool {
        cx.focus_id(self.id, id)
    }

    pub fn title(&self) -> &SharedString {
        &self.title
    }

    pub(crate) fn set_title(&mut self, title: SharedString) {
        self.title = title;
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

    /// Focuses the element keyed `id` (`.id(..)`, or the id a widget constructor takes) once
    /// this frame has rendered, so an element the same handler mounts can take it. When the
    /// element is not focusable itself, its first focusable descendant in tree order does. A
    /// later request in the same frame replaces this one; an id nothing carries is dropped.
    pub fn focus(&mut self, id: impl Into<SharedString>) {
        self.focus_request = Some(id.into());
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
