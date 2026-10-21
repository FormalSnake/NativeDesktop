//! The page in a GPUI window. Chromium's frames are drawn as an image in the page's element, in
//! the window's own frame, so overlays cover the page, it clips with its parents and it moves and
//! resizes with the layout. Every pointer, wheel and key event the element receives goes to
//! Chromium; text and compositions come through GPUI's input handler; `<select>` popups draw over
//! the page and context menus are nd's drawn menus.
//!
//! A Chrome-style page ([`super::chrome`]) is a window of Chromium's over the element instead,
//! which takes its own input and draws itself: the element places it, shapes it around overlays
//! and hands it the keyboard.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use cef::{
    BrowserHost, BrowserSettings, CefString, Client, CompositionUnderline, CompositionUnderlineStyle, CursorType,
    DragData, DragOperationsMask, ImplBrowserHost, ImplMenuModel, KeyEvent, KeyEventType, MenuItemType, MenuModel,
    MouseButtonType, MouseEvent, Rect, RequestContext, RuntimeStyle, WindowInfo, browser_host_create_browser,
};
use nd_backend_gpui::custom::{CustomCx, FocusReporter};
use nd_backend_gpui::{GpuiBackend, MenuRow};
use nd_core::{NodeId, WindowId};
use nd_gpui::gpui::{
    self, AnyElement, AnyWindowHandle, App, AppContext, AsyncApp, Bounds, ContentMask, Corners, CursorStyle,
    DispatchPhase, Element, ElementId, FocusHandle, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId,
    IntoElement, KeyDownEvent, KeyUpEvent, Keystroke, LayoutId, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, RenderImage, ScrollWheelEvent, Style, StyleRefinement, UTF16Selection, Window, fill,
    point, px, size,
};
use nd_webview::WebViewProps;
use nd_webview::menu::Entry;

use super::chrome::{self, Chrome, R};
use super::keys;
use crate::keys::flag;
use crate::menu::Choice;
use crate::page::{self, CefPage};

/// A page's view, as the shared page code holds it.
pub(crate) type View = Rc<OsrView>;

pub(crate) fn new(id: u64) -> View {
    Rc::new(OsrView { id, st: RefCell::default(), chrome: RefCell::default() })
}

/// Chrome style parents Chromium's window to the page's, which takes an X11 window: in a Wayland
/// window the page is off-screen instead.
pub(crate) fn supported(style: nd_webview::CefStyle) -> nd_webview::CefStyle {
    if style == nd_webview::CefStyle::Chrome && !nd_backend_gpui::on_x11() {
        tracing::warn!(
            "nd-cef: Chrome style needs the window on X11; this page is off-screen. Set \
             `[webview.cef] style = \"chrome\"` for the app, which puts its windows on XWayland"
        );
        return nd_webview::CefStyle::Osr;
    }
    style
}

thread_local! {
    /// Pages that painted since the pump last redrew their windows.
    static PAINTED: RefCell<Vec<Weak<OsrView>>> = RefCell::default();
    /// Frames no page shows any more, for the next window that draws to drop from its atlas.
    static RETIRED: RefCell<Vec<Arc<RenderImage>>> = RefCell::default();
}

/// Redraws the windows whose pages painted; the pump calls it after Chromium's work.
pub(crate) fn flush(cx: &mut AsyncApp) {
    let views = PAINTED.with(|p| std::mem::take(&mut *p.borrow_mut()));
    let mut windows: Vec<AnyWindowHandle> = Vec::new();
    for view in views.iter().filter_map(Weak::upgrade) {
        if let Some(handle) = view.st.borrow().window
            && !windows.contains(&handle)
        {
            windows.push(handle);
        }
    }
    for handle in windows {
        let _ = cx.update_window(handle, |_, window, _| window.refresh());
    }
}

/// One frame of the page or its popup, in device pixels.
struct Frame {
    image: Arc<RenderImage>,
    width: u32,
    height: u32,
    /// The colour of its bottom-right pixel.
    edge: gpui::Rgba,
}

/// Where the page is shown, from its last build and layout.
struct Placement {
    backend: GpuiBackend,
    window_id: WindowId,
    node: NodeId,
    reporter: FocusReporter,
}

#[derive(Default)]
struct St {
    frame: Option<Frame>,
    popup: Option<Frame>,
    popup_rect: Option<Rect>,
    popup_visible: bool,
    /// The element's bounds in the window, and the window's scale.
    bounds: Option<Bounds<Pixels>>,
    scale: f32,
    window: Option<AnyWindowHandle>,
    placement: Option<Placement>,
    focus: Option<FocusHandle>,
    focused: bool,
    /// A Chrome-style page holds the X keyboard focus.
    keyboard: bool,
    focus_requested: bool,
    cursor: CursorStyle,
    /// Mouse buttons held, as Chromium's flags.
    held: u32,
    hovered: bool,
    /// Text the input method is composing.
    marked: String,
    /// Chromium's bounds of the composition's characters, in view coordinates.
    composition: Vec<Rect>,
    /// The key-down whose text GPUI's input handler is about to deliver.
    pending: Option<Keystroke>,
    /// The last keystroke the window saw while the page had focus, for [`OsrView::forward_current_key`].
    last_key: Option<Keystroke>,
    interceptor: Option<gpui::Subscription>,
}

pub(crate) struct OsrView {
    id: u64,
    st: RefCell<St>,
    chrome: RefCell<Option<Rc<Chrome>>>,
}

impl OsrView {
    fn page(&self) -> Option<Rc<CefPage>> {
        page::get(self.id)
    }

    fn host(&self) -> Option<BrowserHost> {
        self.page()?.host()
    }

    /// How Chromium creates this view's browser: off-screen, frames as pixel buffers, paced by
    /// Chromium's own begin frames.
    pub(crate) fn window_info(&self) -> WindowInfo {
        WindowInfo { windowless_rendering_enabled: 1, runtime_style: RuntimeStyle::ALLOY, ..WindowInfo::default() }
    }

    /// Frames here are paced by Chromium itself, so input and commands need not wake anything.
    pub(crate) fn wake(&self) {}

    /// Likewise there is no display link of ours to stop when the page leaves its window.
    pub(crate) fn stop_link(&self) {}

    pub(crate) fn chrome(&self) -> Option<Rc<Chrome>> {
        self.chrome.borrow().clone()
    }

    fn become_chrome(&self) -> Result<Rc<Chrome>, String> {
        let chrome = Chrome::new(self.id)?;
        *self.chrome.borrow_mut() = Some(chrome.clone());
        Ok(chrome)
    }

    /// Creates the browser of a Chrome-style page, windowed in a container of this view's.
    pub(crate) fn create_chrome(
        &self,
        mut client: Client,
        settings: &BrowserSettings,
        mut context: Option<RequestContext>,
    ) -> Result<(), String> {
        let info = self.become_chrome()?.window_info();
        // Asynchronous: a windowed browser's creation waits on Chromium's loop, which only runs
        // once this returns. The browser arrives through `on_after_created`.
        let created = browser_host_create_browser(
            Some(&info),
            Some(&mut client),
            Some(&CefString::from("")),
            Some(settings),
            None,
            context.as_mut(),
        );
        if created == 0 {
            return Err("Chromium could not create the browser".into());
        }
        Ok(())
    }

    /// This view's page is the popup a Chrome-style opener is about to create, in a container of
    /// this view's.
    pub(crate) fn expect_chrome_popup(&self, info: &mut WindowInfo) {
        match self.become_chrome() {
            Ok(chrome) => *info = chrome.window_info(),
            Err(e) => tracing::warn!("nd-cef: {e}"),
        }
    }

    /// The browser exists: its window takes the page's place on the next frame.
    pub(crate) fn browser_created(self: &Rc<Self>) {
        if self.chrome().is_some() {
            self.painted();
        }
    }

    pub(crate) fn browser_closed(&self) {}

    /// Whether Chromium can run an extension action on this page: it needs the toolbar model a
    /// Views-hosted browser has, and a page in a native parent window has none (Chromium
    /// dereferences the missing toolbar).
    pub(crate) fn extension_actions(&self) -> Result<(), String> {
        Err("extension actions on Linux need a Views-hosted page, which nd does not create yet".into())
    }

    /// An extension action ran on this page: its popup, if it opens one, goes under `anchor`.
    pub(crate) fn track_extension_popup(&self, id: &str, anchor: nd_webview::extensions::Anchor) {
        if let Some(chrome) = self.chrome() {
            chrome.track_popup(id, anchor);
        }
    }

    pub(crate) fn track_extension_popup_cancel(&self) {
        if let Some(chrome) = self.chrome() {
            chrome.untrack_popup();
        }
    }

    /// A key-down Chromium is about to handle in a Chrome-style page: a chord the app's
    /// bindings take runs there instead, as it would with any other element focused. Returns
    /// whether the app took it.
    pub(crate) fn pre_key(&self, event: &KeyEvent) -> bool {
        if self.chrome().is_none() || event.type_ != KeyEventType::RAWKEYDOWN {
            return false;
        }
        let Some(keystroke) = keys::keystroke(event) else { return false };
        let m = &keystroke.modifiers;
        let chord = m.control || m.alt || m.platform;
        if !chord || crate::keys::edit_command(&keystroke.key, m.shift, m.alt).is_some() {
            return false;
        }
        let (handle, backend) = {
            let st = self.st.borrow();
            (st.window, st.placement.as_ref().map(|p| p.backend.clone()))
        };
        let (Some(handle), Some(mut app)) = (handle, backend.and_then(|b| b.async_app())) else { return false };
        // Busy when Chromium runs from inside an update of GPUI's: the page keeps the key.
        app.update_window(handle, |_, window, cx| window.dispatch_keystroke(keystroke, cx)).unwrap_or(false)
    }

    /// The page closed: its frames go.
    pub(crate) fn detach(&self) {
        if let Some(chrome) = self.chrome() {
            chrome.teardown();
        }
        let mut st = self.st.borrow_mut();
        let frames = [st.frame.take(), st.popup.take()];
        RETIRED.with(|r| r.borrow_mut().extend(frames.into_iter().flatten().map(|f| f.image)));
        st.placement = None;
        st.interceptor = None;
    }

    /// The window the page is in and its backend, for dialogs and menus.
    pub(crate) fn window(&self) -> Option<(WindowId, GpuiBackend)> {
        let st = self.st.borrow();
        st.placement.as_ref().map(|p| (p.window_id, p.backend.clone()))
    }

    /// The view in Chromium's view coordinates: DIPs, top-left origin.
    pub(crate) fn view_rect(&self) -> Rect {
        let st = self.st.borrow();
        let (w, h) = st.bounds.map_or((800., 600.), |b| (b.size.width.as_f32(), b.size.height.as_f32()));
        Rect { x: 0, y: 0, width: (w.round() as i32).max(1), height: (h.round() as i32).max(1) }
    }

    /// GPUI does not say where a window is on screen (Wayland does not tell clients), so the
    /// screen is the view: Chromium keeps `<select>` popups and other widgets inside it, where
    /// they can be drawn.
    pub(crate) fn screen_rect(&self) -> Option<Rect> {
        Some(self.view_rect())
    }

    pub(crate) fn screen_point(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        Some((x, y))
    }

    pub(crate) fn screen_info(&self) -> Option<(f32, Rect)> {
        let scale = self.st.borrow().scale;
        Some((if scale > 0. { scale } else { 1. }, self.view_rect()))
    }

    /// A frame of the page, or of its open `<select>` popup: a BGRA buffer, as GPUI's images are.
    pub(crate) fn present(self: &Rc<Self>, popup: bool, pixels: &[u8], width: usize, height: usize) {
        let last = (width * height - 1) * 4;
        let channel = |i: usize| f32::from(pixels.get(last + i).copied().unwrap_or(255)) / 255.;
        let edge = gpui::Rgba { r: channel(2), g: channel(1), b: channel(0), a: 1. };
        let Some(buffer) = image::RgbaImage::from_raw(width as u32, height as u32, pixels.to_vec()) else { return };
        let frame = Frame {
            image: Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])),
            width: width as u32,
            height: height as u32,
            edge,
        };
        let old = {
            let mut st = self.st.borrow_mut();
            if popup { st.popup.replace(frame) } else { st.frame.replace(frame) }
        };
        if let Some(old) = old {
            RETIRED.with(|r| r.borrow_mut().push(old.image));
        }
        self.painted();
    }

    fn painted(self: &Rc<Self>) {
        PAINTED.with(|p| p.borrow_mut().push(Rc::downgrade(self)));
    }

    pub(crate) fn popup_show(self: &Rc<Self>, show: bool) {
        let old = {
            let mut st = self.st.borrow_mut();
            st.popup_visible = show;
            if show { None } else { st.popup.take() }
        };
        if let Some(old) = old {
            RETIRED.with(|r| r.borrow_mut().push(old.image));
        }
        self.painted();
    }

    pub(crate) fn popup_size(&self, rect: Rect) {
        self.st.borrow_mut().popup_rect = Some(rect);
    }

    pub(crate) fn set_cursor(self: &Rc<Self>, kind: CursorType) {
        self.st.borrow_mut().cursor = cursor_style(kind);
        self.painted();
    }

    pub(crate) fn set_composition_bounds(&self, bounds: Vec<Rect>) {
        self.st.borrow_mut().composition = bounds;
    }

    pub(crate) fn set_selection(&self, _text: String) {}

    /// Dragging out of the page (a link, an image) needs a drag source of the platform's, which
    /// GPUI does not offer: the drag is refused and the page sees it end.
    pub(crate) fn start_dragging(&self, _data: &DragData, _allowed: DragOperationsMask, _x: i32, _y: i32) -> bool {
        false
    }

    pub(crate) fn set_drag_operation(&self, _op: DragOperationsMask) {}

    pub(crate) fn accessibility_update(&self, _change: &serde_json::Value) {}

    pub(crate) fn accessibility_locations(&self, _change: &serde_json::Value) {}

    /// Tab off the page's last field: nd focus moves on past the page.
    pub(crate) fn take_focus(&self, next: bool) {
        let Some((window, backend)) = self.window() else { return };
        let Some(node) = self.st.borrow().placement.as_ref().map(|p| p.node) else { return };
        backend.defer(move |app| {
            let order = app.focus_order(window);
            let Some(at) = order.iter().position(|n| *n == node) else { return };
            let len = order.len();
            let target = if next { order[(at + 1) % len] } else { order[(at + len - 1) % len] };
            app.focus(window, target);
        });
    }

    /// Gives the page GPUI focus on its window's next frame.
    pub(crate) fn focus(self: &Rc<Self>) {
        self.st.borrow_mut().focus_requested = true;
        self.painted();
        super::pump::send(super::pump::Msg::Redraw);
    }

    /// Delivers the key nd is dispatching (Tab, Shift-Tab) to the page. nd's keystroke handling
    /// runs before the window's own interceptors record the key, so this runs a turn later.
    pub(crate) fn forward_current_key(&self) {
        let id = self.id;
        super::run_later(move || {
            let Some(page) = page::get(id) else { return };
            let (Some(host), Some(key)) = (page.host(), page.view.st.borrow().last_key.clone()) else { return };
            host.send_key_event(Some(&keys::key_event(&key, KeyEventType::RAWKEYDOWN, false)));
            let typed = keys::key_event(&key, KeyEventType::CHAR, false);
            if typed.character != 0 {
                host.send_key_event(Some(&typed));
            }
        });
    }

    /// Pops the context menu up at `at` (view coordinates) as nd's drawn menu; `done` hears the
    /// pick, or `None` once it closes without one.
    pub(crate) fn show_menu(
        &self,
        at: (f64, f64),
        model: &MenuModel,
        entries: &[Entry<'_>],
        done: impl FnOnce(Option<Choice>) + 'static,
    ) {
        let Some((window, backend, node)) =
            self.st.borrow().placement.as_ref().map(|p| (p.window_id, p.backend.clone(), p.node))
        else {
            return done(None);
        };
        let mut rows = Vec::new();
        let mut choices = Vec::new();
        add_model(model, &mut rows, &mut choices);
        if !entries.is_empty() {
            if !rows.is_empty() {
                rows.push(MenuRow::Separator);
                choices.push(None);
            }
            add_app(entries, &mut rows, &mut choices);
        }
        if rows.is_empty() {
            return done(None);
        }
        let at = nd_core::Point { x: at.0 as f32, y: at.1 as f32 };
        backend.show_menu(
            window,
            node,
            at,
            rows,
            Box::new(move |row| done(row.and_then(|i| choices.get(i).cloned().flatten()))),
        );
    }

    /// The element that shows the page. Called on every build of its node.
    pub(crate) fn element(self: &Rc<Self>, cx: &CustomCx<'_>, _window: &mut Window) -> AnyElement {
        super::pump::attach_backend(cx.backend());
        let props = cx.props().downcast_ref::<WebViewProps>().expect("webview node needs WebViewProps");
        {
            let mut st = self.st.borrow_mut();
            st.placement = Some(Placement {
                backend: cx.backend().clone(),
                window_id: cx.window_id(),
                node: cx.node(),
                reporter: cx.focus_reporter(),
            });
            if cx.take_focus_request() {
                st.focus_requested = true;
            }
        }
        PageElement { view: self.clone(), node: cx.node(), style: props.style.clone() }.into_any_element()
    }

    fn prepaint(self: &Rc<Self>, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) -> Hitbox {
        for image in RETIRED.with(|r| std::mem::take(&mut *r.borrow_mut())) {
            cx.drop_image(image, Some(window));
        }
        let scale = window.scale_factor();
        let (resized, rescaled, focus, request) = {
            let mut st = self.st.borrow_mut();
            let resized = st.bounds.map(|b| b.size) != Some(bounds.size);
            let rescaled = st.scale != scale;
            st.bounds = Some(bounds);
            st.scale = scale;
            st.window = Some(window.window_handle());
            let focus = st.focus.get_or_insert_with(|| cx.focus_handle()).clone();
            (resized, rescaled, focus, std::mem::take(&mut st.focus_requested))
        };
        if self.st.borrow().interceptor.is_none() {
            let (view, watched) = (Rc::downgrade(self), focus.clone());
            let interceptor = cx.intercept_keystrokes(move |event, window, _| {
                if let Some(view) = view.upgrade()
                    && watched.is_focused(window)
                {
                    view.st.borrow_mut().last_key = Some(event.keystroke.clone());
                }
            });
            self.st.borrow_mut().interceptor = Some(interceptor);
        }
        let chrome = self.chrome();
        if let (None, Some(host)) = (&chrome, self.host()) {
            if rescaled {
                host.notify_screen_info_changed();
            }
            if resized || rescaled {
                host.was_resized();
            }
        }
        window.set_focus_handle(&focus, cx);
        if request && !focus.is_focused(window) {
            window.focus(&focus, cx);
        }
        let focused = focus.is_focused(window) || request;
        let changed = std::mem::replace(&mut self.st.borrow_mut().focused, focused) != focused;
        if changed {
            if let Some(host) = self.host() {
                host.set_focus(i32::from(focused));
            }
            if let Some(page) = self.page() {
                page.page.report_focus(focused);
            }
        }
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(self: &Rc<Self>, bounds: Bounds<Pixels>, hitbox: &Hitbox, window: &mut Window, cx: &mut App) {
        if let Some(chrome) = self.chrome() {
            self.place_chrome(&chrome, bounds, window);
        }
        let st = self.st.borrow();
        let scale = st.scale.max(1.);
        // Device-pixel aligned, so the page's pixels land on the screen's.
        let snap = |v: Pixels| px((v.as_f32() * scale).round() / scale);
        let origin = point(snap(bounds.origin.x), snap(bounds.origin.y));
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            // Under a frame that has not caught up with a resize yet: the page's own edge colour,
            // so a flat page reads as one surface while Chromium draws the new size.
            let under = st.frame.as_ref().map_or(gpui::Rgba { r: 1., g: 1., b: 1., a: 1. }, |f| f.edge);
            window.paint_quad(fill(bounds, under));
            if let Some(frame) = &st.frame {
                let at = Bounds::new(origin, size(px(frame.width as f32 / scale), px(frame.height as f32 / scale)));
                let _ = window.paint_image(bounds, at, Corners::default(), frame.image.clone(), 0, false);
            }
            if let (true, Some(popup), Some(rect)) = (st.popup_visible, &st.popup, &st.popup_rect) {
                let at = Bounds::new(
                    point(origin.x + px(rect.x as f32), origin.y + px(rect.y as f32)),
                    size(px(popup.width as f32 / scale), px(popup.height as f32 / scale)),
                );
                let _ = window.paint_image(bounds, at, Corners::default(), popup.image.clone(), 0, false);
            }
        });
        let focus = st.focus.clone().expect("prepaint made the focus handle");
        if self.chrome().is_some() {
            drop(st);
            self.listen_wheel(hitbox, window);
            return;
        }
        window.set_cursor_style(st.cursor, hitbox);
        drop(st);
        self.listen(hitbox, &focus, window);
        window.handle_input(&focus, Input { view: self.clone() }, cx);
    }

    /// Moves the page's window to the element, shapes it around this frame's overlays, and
    /// fills the holes they cut with a snapshot of the page, which the element paints.
    fn place_chrome(self: &Rc<Self>, chrome: &Rc<Chrome>, bounds: Bounds<Pixels>, window: &mut Window) {
        let Some(parent) = x11_window(window) else { return };
        let scale = window.scale_factor();
        let device = |b: Bounds<Pixels>| {
            let (x, y) = ((b.origin.x.as_f32() * scale).round() as i32, (b.origin.y.as_f32() * scale).round() as i32);
            let right = ((b.origin.x + b.size.width).as_f32() * scale).round() as i32;
            let bottom = ((b.origin.y + b.size.height).as_f32() * scale).round() as i32;
            R::new(x, y, right - x, bottom - y)
        };
        let frame = device(bounds);
        let visible = device(window.content_mask().bounds).intersect(frame);
        let overlays: Vec<(R, bool, bool)> = nd_backend_gpui::occlusion::occluders(window)
            .iter()
            .map(|o| (device(o.bounds), o.draws, o.blocks_input))
            .collect();
        let shape = chrome::shape(frame, visible, &overlays, scale);
        let cut = shape.cut;
        let had = self.st.borrow().frame.is_some();
        // The snapshot is taken before the overlay's hole is cut, while the page still shows.
        if cut
            && !had
            && let Some((pixels, width, height)) = chrome.snapshot()
        {
            self.set_frame(pixels, width, height);
        }
        let resized = chrome.place(parent, frame, shape);
        // An overlay that takes the pointer takes the keys too, as it does over any other element.
        let keyboard = self.st.borrow().focused && !overlays.iter().any(|&(_, _, blocks)| blocks);
        if std::mem::replace(&mut self.st.borrow_mut().keyboard, keyboard) != keyboard {
            chrome.focus(keyboard);
        }
        if had
            && (!cut || resized)
            && let Some(old) = self.st.borrow_mut().frame.take()
        {
            RETIRED.with(|r| r.borrow_mut().push(old.image));
        }
        let view = Rc::downgrade(self);
        nd_backend_gpui::occlusion::shown(window, self.id, move || {
            let Some(view) = view.upgrade() else { return };
            view.st.borrow_mut().keyboard = false;
            if let Some(chrome) = view.chrome() {
                chrome.park();
            }
        });
    }

    fn set_frame(&self, pixels: Vec<u8>, width: u32, height: u32) {
        let Some(buffer) = image::RgbaImage::from_raw(width, height, pixels) else { return };
        let frame = Frame {
            image: Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])),
            width,
            height,
            edge: gpui::Rgba { r: 1., g: 1., b: 1., a: 1. },
        };
        if let Some(old) = self.st.borrow_mut().frame.replace(frame) {
            RETIRED.with(|r| r.borrow_mut().push(old.image));
        }
    }

    /// A pointer position as Chromium's mouse event, in the view's coordinates.
    fn mouse(&self, position: Point<Pixels>, modifiers: &Modifiers) -> MouseEvent {
        let st = self.st.borrow();
        let origin = st.bounds.map_or(Point::default(), |b| b.origin);
        MouseEvent {
            x: (position.x - origin.x).as_f32().round() as i32,
            y: (position.y - origin.y).as_f32().round() as i32,
            modifiers: keys::modifiers(modifiers) | st.held,
        }
    }

    fn listen(self: &Rc<Self>, hitbox: &Hitbox, focus: &FocusHandle, window: &mut Window) {
        let (view, hit, focus_down) = (Rc::downgrade(self), hitbox.clone(), focus.clone());
        window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
            let Some(view) = view.upgrade() else { return };
            if phase != DispatchPhase::Bubble || !hit.is_hovered(window) {
                return;
            }
            if !focus_down.is_focused(window) {
                window.focus(&focus_down, cx);
                let reporter = view.st.borrow().placement.as_ref().map(|p| p.reporter.clone());
                if let Some(reporter) = reporter {
                    reporter.focused(cx);
                }
            }
            let Some((button, bit)) = button(e.button) else { return };
            let Some(host) = view.host() else { return };
            view.st.borrow_mut().held |= bit;
            let mouse = view.mouse(e.position, &e.modifiers);
            host.send_mouse_click_event(Some(&mouse), button, 0, e.click_count.max(1) as i32);
        });
        let view = Rc::downgrade(self);
        window.on_mouse_event(move |e: &MouseUpEvent, phase, _, _| {
            let Some(view) = view.upgrade() else { return };
            let Some((button, bit)) = button(e.button) else { return };
            if phase != DispatchPhase::Bubble || view.st.borrow().held & bit == 0 {
                return;
            }
            let Some(host) = view.host() else { return };
            let mouse = view.mouse(e.position, &e.modifiers);
            view.st.borrow_mut().held &= !bit;
            host.send_mouse_click_event(Some(&mouse), button, 1, e.click_count.max(1) as i32);
        });
        let (view, hit) = (Rc::downgrade(self), hitbox.clone());
        window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, _| {
            let Some(view) = view.upgrade() else { return };
            if phase != DispatchPhase::Bubble {
                return;
            }
            let hovered = hit.is_hovered(window);
            let (was, held) = {
                let mut st = view.st.borrow_mut();
                (std::mem::replace(&mut st.hovered, hovered), st.held)
            };
            if !hovered && held == 0 && !was {
                return;
            }
            let Some(host) = view.host() else { return };
            let mouse = view.mouse(e.position, &e.modifiers);
            host.send_mouse_move_event(Some(&mouse), i32::from(!hovered && held == 0));
        });
        self.listen_wheel(hitbox, window);
        let (view, focused) = (Rc::downgrade(self), focus.clone());
        window.on_key_event(move |e: &KeyDownEvent, phase, window, cx| {
            let Some(view) = view.upgrade() else { return };
            if phase != DispatchPhase::Bubble || !focused.is_focused(window) {
                return;
            }
            if view.key_down(e) {
                cx.stop_propagation();
            }
        });
        let (view, focused) = (Rc::downgrade(self), focus.clone());
        window.on_key_event(move |e: &KeyUpEvent, phase, window, _| {
            let Some(view) = view.upgrade() else { return };
            if phase != DispatchPhase::Bubble || !focused.is_focused(window) {
                return;
            }
            if let Some(host) = view.host() {
                host.send_key_event(Some(&keys::key_event(&e.keystroke, KeyEventType::KEYUP, false)));
            }
        });
    }

    /// The wheel over the page goes to Chromium, also when a Chrome-style page's window lets it
    /// through to here (under a light-dismiss overlay's backdrop).
    fn listen_wheel(self: &Rc<Self>, hitbox: &Hitbox, window: &mut Window) {
        let (view, hit) = (Rc::downgrade(self), hitbox.clone());
        window.on_mouse_event(move |e: &ScrollWheelEvent, phase, window, cx| {
            let Some(view) = view.upgrade() else { return };
            if phase != DispatchPhase::Bubble || !hit.should_handle_scroll(window) {
                return;
            }
            let Some(host) = view.host() else { return };
            let (dx, dy, precise) = keys::wheel_delta(&e.delta);
            let mut mouse = view.mouse(e.position, &e.modifiers);
            if precise {
                mouse.modifiers |= flag::PRECISION_SCROLLING_DELTA;
            }
            host.send_mouse_wheel_event(Some(&mouse), dx, dy);
            cx.stop_propagation();
        });
    }

    /// A key-down for the page. Returns whether it was handled whole; a key that types text
    /// leaves that to GPUI's input handler, which hears it next.
    fn key_down(&self, e: &KeyDownEvent) -> bool {
        let Some(host) = self.host() else { return false };
        if !self.st.borrow().marked.is_empty() {
            return false;
        }
        let k = &e.keystroke;
        host.send_key_event(Some(&keys::key_event(k, KeyEventType::RAWKEYDOWN, e.is_held)));
        if k.key_char.is_some() && k.modifiers.is_subset_of(&Modifiers::shift()) {
            self.st.borrow_mut().pending = Some(k.clone());
            return false;
        }
        let typed = keys::key_event(k, KeyEventType::CHAR, e.is_held);
        if typed.character != 0 {
            host.send_key_event(Some(&typed));
        }
        true
    }
}

/// The X11 window GPUI draws `window` into.
fn x11_window(window: &Window) -> Option<u32> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        RawWindowHandle::Xlib(h) => u32::try_from(h.window).ok(),
        _ => None,
    }
}

fn button(button: MouseButton) -> Option<(MouseButtonType, u32)> {
    match button {
        MouseButton::Left => Some((MouseButtonType::LEFT, flag::LEFT_MOUSE)),
        MouseButton::Middle => Some((MouseButtonType::MIDDLE, flag::MIDDLE_MOUSE)),
        MouseButton::Right => Some((MouseButtonType::RIGHT, flag::RIGHT_MOUSE)),
        MouseButton::Navigate(_) => None,
    }
}

fn cursor_style(kind: CursorType) -> CursorStyle {
    match kind {
        CursorType::HAND => CursorStyle::PointingHand,
        CursorType::IBEAM => CursorStyle::IBeam,
        CursorType::VERTICALTEXT => CursorStyle::IBeamCursorForVerticalLayout,
        CursorType::CROSS | CursorType::CELL => CursorStyle::Crosshair,
        CursorType::EASTRESIZE => CursorStyle::ResizeRight,
        CursorType::WESTRESIZE => CursorStyle::ResizeLeft,
        CursorType::NORTHRESIZE => CursorStyle::ResizeUp,
        CursorType::SOUTHRESIZE => CursorStyle::ResizeDown,
        CursorType::EASTWESTRESIZE => CursorStyle::ResizeLeftRight,
        CursorType::NORTHSOUTHRESIZE => CursorStyle::ResizeUpDown,
        CursorType::NORTHEASTRESIZE | CursorType::SOUTHWESTRESIZE | CursorType::NORTHEASTSOUTHWESTRESIZE => {
            CursorStyle::ResizeUpRightDownLeft
        }
        CursorType::NORTHWESTRESIZE | CursorType::SOUTHEASTRESIZE | CursorType::NORTHWESTSOUTHEASTRESIZE => {
            CursorStyle::ResizeUpLeftDownRight
        }
        CursorType::COLUMNRESIZE => CursorStyle::ResizeColumn,
        CursorType::ROWRESIZE => CursorStyle::ResizeRow,
        CursorType::GRAB | CursorType::MOVE => CursorStyle::OpenHand,
        CursorType::GRABBING => CursorStyle::ClosedHand,
        CursorType::NOTALLOWED | CursorType::NODROP | CursorType::DND_NONE => CursorStyle::OperationNotAllowed,
        CursorType::ALIAS | CursorType::DND_LINK => CursorStyle::DragLink,
        CursorType::COPY | CursorType::DND_COPY => CursorStyle::DragCopy,
        CursorType::CONTEXTMENU => CursorStyle::ContextualMenu,
        _ => CursorStyle::Arrow,
    }
}

/// Chromium's menu model as rows, submenus inline after a separator (the drawn menu has no
/// cascades), with what each row picks.
fn add_model(model: &MenuModel, rows: &mut Vec<MenuRow>, choices: &mut Vec<Option<Choice>>) {
    for i in 0..model.count() {
        if model.is_visible_at(i) == 0 {
            continue;
        }
        let kind = model.type_at(i);
        if kind == MenuItemType::SEPARATOR {
            separator(rows, choices);
            continue;
        }
        if kind == MenuItemType::SUBMENU {
            if let Some(sub) = model.sub_menu_at(i) {
                separator(rows, choices);
                add_model(&sub, rows, choices);
                separator(rows, choices);
            }
            continue;
        }
        let label = crate::menu::label(&crate::text(&model.label_at(i)));
        rows.push(MenuRow::Item {
            label: label.into(),
            enabled: model.is_enabled_at(i) != 0,
            checked: model.is_checked_at(i) != 0,
        });
        choices.push(Some(Choice::Chromium(model.command_id_at(i))));
    }
    if matches!(rows.last(), Some(MenuRow::Separator)) {
        rows.pop();
        choices.pop();
    }
}

fn add_app(entries: &[Entry<'_>], rows: &mut Vec<MenuRow>, choices: &mut Vec<Option<Choice>>) {
    for entry in entries {
        match entry {
            Entry::Separator => separator(rows, choices),
            Entry::Submenu(_, children) => {
                separator(rows, choices);
                add_app(children, rows, choices);
                separator(rows, choices);
            }
            Entry::Item(item) => {
                rows.push(MenuRow::Item {
                    label: item.label.clone().into(),
                    enabled: item.enabled,
                    checked: item.checked,
                });
                choices.push(Some(Choice::App((*item).clone())));
            }
        }
    }
}

/// A separator, never two in a row or one at the top.
fn separator(rows: &mut Vec<MenuRow>, choices: &mut Vec<Option<Choice>>) {
    if !matches!(rows.last(), None | Some(MenuRow::Separator)) {
        rows.push(MenuRow::Separator);
        choices.push(None);
    }
}

struct PageElement {
    view: View,
    node: NodeId,
    style: StyleRefinement,
}

impl IntoElement for PageElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for PageElement {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::Integer(self.node.0))
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        gpui::Refineable::refine(&mut style, &self.style);
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        self.view.prepaint(bounds, window, cx)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.view.paint(bounds, hitbox, window, cx);
    }
}

/// Text input for the focused page: committed text and compositions go to Chromium's IME API.
struct Input {
    view: View,
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

impl gpui::InputHandler for Input {
    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut App) -> Option<UTF16Selection> {
        let len = utf16_len(&self.view.st.borrow().marked);
        Some(UTF16Selection { range: len..len, reversed: false })
    }

    fn marked_text_range(&mut self, _: &mut Window, _: &mut App) -> Option<Range<usize>> {
        let st = self.view.st.borrow();
        (!st.marked.is_empty()).then(|| 0..utf16_len(&st.marked))
    }

    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<String> {
        let units: Vec<u16> = self.view.st.borrow().marked.encode_utf16().collect();
        let end = range.end.min(units.len());
        let start = range.start.min(end);
        *adjusted = Some(start..end);
        String::from_utf16(&units[start..end]).ok()
    }

    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, _: &mut App) {
        let Some(host) = self.view.host() else { return };
        let (composing, pending) = {
            let mut st = self.view.st.borrow_mut();
            (!std::mem::take(&mut st.marked).is_empty(), st.pending.take())
        };
        let mut units = text.encode_utf16();
        let single = match (units.next(), units.next()) {
            (Some(unit), None) => Some(unit),
            _ => None,
        };
        match (composing, pending, single) {
            // The text of the key just pressed: a keypress of that key.
            (false, Some(key), Some(unit)) => {
                let mut typed = keys::key_event(&key, KeyEventType::CHAR, false);
                typed.character = unit;
                typed.unmodified_character = unit;
                host.send_key_event(Some(&typed));
            }
            _ => host.ime_commit_text(Some(&CefString::from(text)), None, 0),
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        _: &mut App,
    ) {
        self.view.st.borrow_mut().marked = text.to_owned();
        let Some(host) = self.view.host() else { return };
        if text.is_empty() {
            host.ime_cancel_composition();
            return;
        }
        let len = utf16_len(text) as u32;
        let underline = CompositionUnderline {
            range: cef::Range { from: 0, to: len },
            color: 0xff00_0000,
            background_color: 0,
            thick: 0,
            style: CompositionUnderlineStyle::SOLID,
            ..CompositionUnderline::default()
        };
        let selection = selected
            .map_or(cef::Range { from: len, to: len }, |r| cef::Range { from: r.start as u32, to: r.end as u32 });
        host.ime_set_composition(Some(&CefString::from(text)), Some(&[underline]), None, Some(&selection));
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut App) {
        let had = !std::mem::take(&mut self.view.st.borrow_mut().marked).is_empty();
        if let (true, Some(host)) = (had, self.view.host()) {
            host.ime_finish_composing_text(0);
        }
    }

    /// Where the input method's candidate window goes: under the composition's characters.
    fn bounds_for_range(&mut self, range: Range<usize>, _: &mut Window, _: &mut App) -> Option<Bounds<Pixels>> {
        let st = self.view.st.borrow();
        let origin = st.bounds?.origin;
        let rect = st.composition.get(range.start.min(st.composition.len().saturating_sub(1)))?;
        Some(Bounds::new(
            point(origin.x + px(rect.x as f32), origin.y + px(rect.y as f32)),
            size(px(rect.width as f32), px(rect.height as f32)),
        ))
    }

    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut App) -> Option<usize> {
        None
    }

    fn apple_press_and_hold_enabled(&mut self) -> bool {
        false
    }
}
