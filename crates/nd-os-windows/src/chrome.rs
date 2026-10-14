//! Window chrome on a backend's HWND: caption hit-testing, app-drawn caption buttons that behave
//! as the system's (Snap Layouts included), drag regions, and the `WM_POINTER` pen and touch path.
//!
//! Installed as a comctl32 window subclass, so it runs before the backend's window procedure and
//! needs nothing from it. It answers `WM_NCHITTEST` from the point in `lParam`. GPUI answers it
//! from the last mouse move it saw, which lags one move behind: the message arrives before the
//! move it belongs to, so the first hover over a button misses and the Snap Layouts flyout never
//! opens on a fast approach.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTCLOSE, HTLEFT, HTMAXBUTTON, HTMINBUTTON, HTTOP, HTTOPLEFT, HTTOPRIGHT,
    IsZoomed, PostMessageW, SM_CXPADDEDBORDER, SM_CYFRAME, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, ShowWindowAsync,
    WM_CLOSE, WM_DPICHANGED, WM_MOUSEMOVE, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN,
    WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_POINTERDOWN, WM_POINTERUP, WM_POINTERUPDATE,
};

use crate::caption::{CaptionButton, CaptionLayout, CaptionState, Edge, Hit};
use crate::pointer::{self, PointerEvent, PointerKind};

/// Subclass id; one chrome per window.
const SUBCLASS_ID: usize = 0x6e64_6368;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChromeEvent {
    /// Hover or press on a drawn caption button changed: repaint the title row.
    Caption(CaptionState),
    /// A pen or touch contact, with the axes the promoted mouse event drops.
    Pointer(PointerEvent),
    /// The window moved to a monitor with another scale.
    Scale(f32),
}

type Handler = Box<dyn FnMut(ChromeEvent)>;

struct Shared {
    layout: RefCell<CaptionLayout>,
    state: Cell<CaptionState>,
    scale: Cell<f32>,
    installed: Cell<bool>,
    on_event: RefCell<Option<Handler>>,
}

impl Shared {
    fn emit(&self, event: ChromeEvent) {
        // Taken out while it runs, so a handler that calls back into the chrome finds no borrow.
        let Some(mut handler) = self.on_event.borrow_mut().take() else { return };
        handler(event);
        let mut slot = self.on_event.borrow_mut();
        if slot.is_none() {
            *slot = Some(handler);
        }
    }

    fn set_state(&self, state: CaptionState) {
        if self.state.replace(state) != state {
            self.emit(ChromeEvent::Caption(state));
        }
    }

    /// What the screen point in `lparam` hits.
    fn hit(&self, hwnd: HWND, lparam: LPARAM) -> Hit {
        let mut at = POINT { x: (lparam.0 & 0xffff) as i16 as i32, y: ((lparam.0 >> 16) & 0xffff) as i16 as i32 };
        let _ = unsafe { ScreenToClient(hwnd, &mut at) };
        let scale = self.scale.get();
        let dpi = (scale * 96.).round() as u32;
        let band = unsafe { GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi) };
        let maximized = unsafe { IsZoomed(hwnd) }.as_bool();
        self.layout.borrow().hit(at.x as f32 / scale, at.y as f32 / scale, maximized, band as f32 / scale)
    }
}

fn hit_code(hit: Hit) -> u32 {
    match hit {
        Hit::Client => HTCLIENT,
        Hit::Caption => HTCAPTION,
        Hit::Button(CaptionButton::Minimize) => HTMINBUTTON,
        Hit::Button(CaptionButton::Maximize) => HTMAXBUTTON,
        Hit::Button(CaptionButton::Close) => HTCLOSE,
        Hit::Resize(Edge::Top) => HTTOP,
        Hit::Resize(Edge::TopLeft) => HTTOPLEFT,
        Hit::Resize(Edge::TopRight) => HTTOPRIGHT,
    }
}

fn button_from_code(code: usize) -> Option<CaptionButton> {
    match code as u32 {
        HTMINBUTTON => Some(CaptionButton::Minimize),
        HTMAXBUTTON => Some(CaptionButton::Maximize),
        HTCLOSE => Some(CaptionButton::Close),
        _ => None,
    }
}

/// The chrome of one window. Dropping it removes the subclass.
pub struct Chrome {
    hwnd: HWND,
    shared: Rc<Shared>,
}

impl Chrome {
    /// Installs on the window behind `window` (a GPUI `Window`, a winit window, any
    /// `raw-window-handle` source).
    pub fn install(window: &impl HasWindowHandle, layout: CaptionLayout) -> Result<Self, String> {
        let handle = window.window_handle().map_err(|e| format!("window handle: {e}"))?;
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return Err("not a Win32 window".into());
        };
        Self::install_hwnd(HWND(win32.hwnd.get() as *mut _), layout)
    }

    pub fn install_hwnd(hwnd: HWND, layout: CaptionLayout) -> Result<Self, String> {
        let shared = Rc::new(Shared {
            layout: RefCell::new(layout),
            state: Cell::new(CaptionState::default()),
            scale: Cell::new(crate::dpi::scale(hwnd)),
            installed: Cell::new(true),
            on_event: RefCell::new(None),
        });
        let data = Rc::into_raw(shared.clone()) as usize;
        if !unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, data) }.as_bool() {
            drop(unsafe { Rc::from_raw(data as *const Shared) });
            return Err("SetWindowSubclass failed".into());
        }
        Ok(Self { hwnd, shared })
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Replaces the title row; call after every layout pass that moved it. An empty layout (no
    /// title row, no buttons) hands the whole window to the app, as full screen needs.
    pub fn set_layout(&self, layout: CaptionLayout) {
        *self.shared.layout.borrow_mut() = layout;
    }

    pub fn layout(&self) -> CaptionLayout {
        self.shared.layout.borrow().clone()
    }

    pub fn caption_state(&self) -> CaptionState {
        self.shared.state.get()
    }

    pub fn scale(&self) -> f32 {
        self.shared.scale.get()
    }

    /// Called from the window procedure for posted input (non-client mouse, pointer) and DPI
    /// changes. It must not block, and it runs while no backend borrow is held only for those
    /// messages, which is the case for every event it receives.
    pub fn on_event(&self, handler: impl FnMut(ChromeEvent) + 'static) {
        *self.shared.on_event.borrow_mut() = Some(Box::new(handler));
    }
}

impl Drop for Chrome {
    fn drop(&mut self) {
        if self.shared.installed.replace(false) {
            unsafe {
                let _ = RemoveWindowSubclass(self.hwnd, Some(subclass_proc), SUBCLASS_ID);
                drop(Rc::from_raw(Rc::as_ptr(&self.shared)));
            }
        }
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: `data` is the `Rc<Shared>` leaked at install, released only on WM_NCDESTROY or by
    // `Chrome::drop`, both of which remove this subclass first.
    let shared = unsafe { &*(data as *const Shared) };
    let forward = || unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    match msg {
        WM_NCHITTEST => {
            // Side and bottom borders stay the system frame's; the backend may also own the top band.
            let inner = forward();
            if (HTLEFT..=HTBOTTOMRIGHT).contains(&(inner.0 as u32)) {
                return inner;
            }
            match shared.hit(hwnd, lparam) {
                // Content and the backend's own drag regions: its answer stands.
                Hit::Client => inner,
                hit => LRESULT(hit_code(hit) as isize),
            }
        }
        WM_NCMOUSEMOVE => {
            let hovered = match shared.hit(hwnd, lparam) {
                Hit::Button(button) => Some(button),
                _ => None,
            };
            let state = shared.state.get();
            shared.set_state(CaptionState { hovered, pressed: state.pressed.filter(|p| Some(*p) == hovered) });
            forward()
        }
        WM_NCMOUSELEAVE => {
            shared.set_state(CaptionState::default());
            forward()
        }
        WM_MOUSEMOVE => {
            if shared.state.get() != CaptionState::default() {
                shared.set_state(CaptionState::default());
            }
            forward()
        }
        // Eaten: DefWindowProc would draw a classic button and run its own tracking loop.
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if button_from_code(wparam.0).is_some() => {
            let button = button_from_code(wparam.0);
            shared.set_state(CaptionState { hovered: button, pressed: button });
            LRESULT(0)
        }
        WM_NCLBUTTONUP if button_from_code(wparam.0).is_some() => {
            let button = button_from_code(wparam.0);
            let pressed = shared.state.get().pressed;
            shared.set_state(CaptionState { hovered: button, pressed: None });
            if pressed == button {
                unsafe {
                    match button {
                        Some(CaptionButton::Minimize) => {
                            let _ = ShowWindowAsync(hwnd, SW_MINIMIZE);
                        }
                        Some(CaptionButton::Maximize) => {
                            let zoomed = IsZoomed(hwnd).as_bool();
                            let _ = ShowWindowAsync(hwnd, if zoomed { SW_RESTORE } else { SW_MAXIMIZE });
                        }
                        Some(CaptionButton::Close) => {
                            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                        }
                        None => {}
                    }
                }
            }
            LRESULT(0)
        }
        WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP => {
            if let Some(event) = pointer::read(hwnd, msg, wparam, shared.scale.get())
                && matches!(event.kind, PointerKind::Pen | PointerKind::Touch)
            {
                shared.emit(ChromeEvent::Pointer(event));
            }
            // DefWindowProc promotes the pointer to mouse messages, which the backend handles.
            forward()
        }
        WM_DPICHANGED => {
            // HIWORD(wParam) is the new Y DPI.
            let scale = ((wparam.0 >> 16) & 0xffff) as f32 / 96.;
            if scale > 0. && shared.scale.replace(scale) != scale {
                shared.emit(ChromeEvent::Scale(scale));
            }
            forward()
        }
        WM_NCDESTROY => {
            if shared.installed.replace(false) {
                unsafe {
                    let _ = RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID);
                    drop(Rc::from_raw(data as *const Shared));
                }
            }
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
        _ => forward(),
    }
}
