//! Chrome-style pages on Linux: a windowed browser in an X11 child window of the page's GPUI
//! window, which therefore runs on X11 (XWayland under a Wayland compositor:
//! `nd_backend_gpui::prefer_x11`). Chromium parents its own X window to a container of ours, so
//! the page moves, resizes, hides and clips by moving, resizing, unmapping and shaping the
//! container.
//!
//! The X server stacks a child window above everything its parent draws, so GPUI's overlays
//! would sit under the page: the container's bounding shape leaves out what the overlay layer
//! draws this frame (`nd_backend_gpui::occlusion`), its input shape leaves out what the layer
//! takes the pointer for, and a snapshot of the page fills the holes so the overlay draws over
//! the page instead of over nothing.
//!
//! The container is created parked under the root window, unmapped, since Chromium needs its
//! parent before the page is in any window; it moves into the GPUI window when the page is first
//! drawn, and back to the root when the page closes, so Chromium's window outlives neither the
//! page nor its own close.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use cef::{RuntimeStyle, WindowInfo};
use x11rb::connection::Connection;
use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
use x11rb::protocol::xproto::{
    ClipOrdering, ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, ImageFormat, InputFocus, Rectangle,
    StackMode, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

/// Shadows and rounded corners of an overlay reach past its bounds: the hole it cuts in the page
/// is this much larger, in device pixels at scale 1.
const SHADOW: i32 = 12;

/// A rectangle in device pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct R {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl R {
    pub(crate) fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    fn right(self) -> i32 {
        self.x + self.w
    }

    fn bottom(self) -> i32 {
        self.y + self.h
    }

    fn is_empty(self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub(crate) fn intersect(self, o: R) -> Option<R> {
        let (x, y) = (self.x.max(o.x), self.y.max(o.y));
        let r = R::new(x, y, self.right().min(o.right()) - x, self.bottom().min(o.bottom()) - y);
        (!r.is_empty()).then_some(r)
    }

    pub(crate) fn offset(self, dx: i32, dy: i32) -> R {
        R::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    pub(crate) fn inflate(self, by: i32) -> R {
        R::new(self.x - by, self.y - by, self.w + 2 * by, self.h + 2 * by)
    }
}

/// `region` without `hole`, as disjoint rectangles: each one it overlaps splits into the bands
/// above and below the hole and the pieces left and right of it.
pub(crate) fn subtract(region: &[R], hole: R) -> Vec<R> {
    let mut out = Vec::with_capacity(region.len() + 3);
    for &r in region {
        let Some(i) = r.intersect(hole) else {
            out.push(r);
            continue;
        };
        let pieces = [
            R::new(r.x, r.y, r.w, i.y - r.y),
            R::new(r.x, i.bottom(), r.w, r.bottom() - i.bottom()),
            R::new(r.x, i.y, i.x - r.x, i.h),
            R::new(i.right(), i.y, r.right() - i.right(), i.h),
        ];
        out.extend(pieces.into_iter().filter(|p| !p.is_empty()));
    }
    out
}

/// What of the container shows and what takes the pointer, in its own coordinates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Shape {
    pub bounding: Vec<R>,
    pub input: Vec<R>,
    /// Some overlay cuts into what would show.
    pub cut: bool,
}

/// The shape of a page whose container covers `frame` and shows `visible` of it (both in window
/// coordinates), under overlays given as (bounds, draws, takes the pointer) at `scale`.
pub(crate) fn shape(frame: R, visible: Option<R>, overlays: &[(R, bool, bool)], scale: f32) -> Shape {
    let Some(visible) = visible.and_then(|v| v.intersect(frame)) else { return Shape::default() };
    let pad = (SHADOW as f32 * scale).round() as i32;
    let mut bounding = vec![visible];
    let mut cut = false;
    for &(bounds, draws, _) in overlays {
        if draws && bounds.inflate(pad).intersect(visible).is_some() {
            bounding = subtract(&bounding, bounds.inflate(pad));
            cut = true;
        }
    }
    let mut input = bounding.clone();
    for &(bounds, _, blocks) in overlays {
        if blocks {
            input = subtract(&input, bounds);
        }
    }
    let local = |rs: Vec<R>| rs.into_iter().map(|r| r.offset(-frame.x, -frame.y)).collect();
    Shape { bounding: local(bounding), input: local(input), cut }
}

thread_local! {
    static CONN: RefCell<Option<Rc<X>>> = const { RefCell::new(None) };
    /// Containers of closed pages, until Chromium's browser in each is gone.
    static RELEASED: RefCell<HashMap<u64, Window>> = RefCell::default();
    static LIVE: Cell<usize> = const { Cell::new(0) };
}

/// nd-cef's own connection to the X server the app's windows are on.
struct X {
    conn: RustConnection,
    root: Window,
    depth: u8,
    visual: u32,
    colormap: u32,
    white: u32,
}

fn x() -> Result<Rc<X>, String> {
    CONN.with(|c| {
        if let Some(x) = c.borrow().as_ref() {
            return Ok(x.clone());
        }
        let (conn, screen) = RustConnection::connect(None).map_err(|e| format!("X11: {e}"))?;
        let s = &conn.setup().roots[screen];
        let x = Rc::new(X {
            root: s.root,
            depth: s.root_depth,
            visual: s.root_visual,
            colormap: s.default_colormap,
            white: s.white_pixel,
            conn,
        });
        *c.borrow_mut() = Some(x.clone());
        Ok(x)
    })
}

impl X {
    /// Sends what is queued, and drops the errors of requests nobody waits on (a focus change
    /// to a window that just unmapped).
    fn flush(&self) {
        let _ = self.conn.flush();
        while let Ok(Some(_)) = self.conn.poll_for_event() {}
    }
}

/// Chrome-style pages open: Chromium's X events then need its loop run often (`pump`).
pub(crate) fn live() -> usize {
    LIVE.with(Cell::get)
}

#[derive(Default)]
struct St {
    /// The GPUI window the container is in; the root while parked.
    parent: Window,
    mapped: bool,
    frame: R,
    shape: Option<Shape>,
    /// Chromium's window and the size it was given: it only exists once the browser does, and
    /// does not follow the container on its own.
    browser: Option<(Window, i32, i32)>,
}

pub(crate) struct Chrome {
    page: u64,
    x: Rc<X>,
    container: Window,
    st: RefCell<St>,
}

impl Chrome {
    pub(crate) fn new(page: u64) -> Result<Rc<Self>, String> {
        let x = x()?;
        let container = x.conn.generate_id().map_err(|e| format!("X11: {e}"))?;
        // The screen's own visual, not the parent's: a GPUI window can be 32-bit ARGB, and
        // Chromium's GL surface cannot be parented under one.
        let aux = CreateWindowAux::new()
            .background_pixel(x.white)
            .border_pixel(0)
            .colormap(x.colormap)
            .override_redirect(1);
        x.conn
            .create_window(x.depth, container, x.root, 0, 0, 800, 600, 0, WindowClass::INPUT_OUTPUT, x.visual, &aux)
            .map_err(|e| format!("X11: {e}"))?;
        x.conn.sync().map_err(|e| format!("X11: {e}"))?;
        LIVE.with(|l| l.set(l.get() + 1));
        Ok(Rc::new(Self { page, x: x.clone(), container, st: RefCell::new(St { parent: x.root, ..St::default() }) }))
    }

    /// How Chromium creates the page's browser: windowed, in the container, Chrome style.
    pub(crate) fn window_info(&self) -> WindowInfo {
        WindowInfo {
            parent_window: u64::from(self.container) as _,
            bounds: cef::Rect { x: 0, y: 0, width: 800, height: 600 },
            runtime_style: RuntimeStyle::CHROME,
            ..WindowInfo::default()
        }
    }

    fn browser_window(&self) -> Option<Window> {
        let host = crate::page::get(self.page)?.host()?;
        u32::try_from(cef::ImplBrowserHost::window_handle(&host)).ok().filter(|w| *w != 0)
    }

    /// The page is drawn at `frame` of GPUI window `parent` with `shape`. Returns whether the
    /// container or Chromium's window changed size, so a snapshot taken before is stale.
    pub(crate) fn place(&self, parent: Window, frame: R, shape: Shape) -> bool {
        let conn = &self.x.conn;
        let mut st = self.st.borrow_mut();
        if st.parent != parent {
            let _ = conn.reparent_window(self.container, parent, frame.x as i16, frame.y as i16);
            st.parent = parent;
            st.frame = R::default();
            st.mapped = false;
        }
        let mut resized = st.frame.w != frame.w || st.frame.h != frame.h;
        if st.frame != frame {
            let aux = ConfigureWindowAux::new()
                .x(frame.x)
                .y(frame.y)
                .width(frame.w.max(1) as u32)
                .height(frame.h.max(1) as u32)
                .stack_mode(StackMode::ABOVE);
            let _ = conn.configure_window(self.container, &aux);
            st.frame = frame;
        }
        let browser = self.browser_window().map(|w| (w, frame.w, frame.h));
        if let Some((window, w, h)) = browser
            && st.browser != browser
        {
            let aux = ConfigureWindowAux::new().x(0).y(0).width(w.max(1) as u32).height(h.max(1) as u32);
            let _ = conn.configure_window(window, &aux);
            st.browser = browser;
            resized = true;
        }
        if st.shape.as_ref() != Some(&shape) {
            let rects = |rs: &[R]| -> Vec<Rectangle> {
                rs.iter()
                    .map(|r| Rectangle { x: r.x as i16, y: r.y as i16, width: r.w as u16, height: r.h as u16 })
                    .collect()
            };
            let whole = [R::new(0, 0, frame.w, frame.h)];
            for (kind, region) in [(SK::BOUNDING, &shape.bounding), (SK::INPUT, &shape.input)] {
                let region = if region.as_slice() == whole { None } else { Some(rects(region)) };
                let _ = match region {
                    None => conn.shape_mask(SO::SET, kind, self.container, 0, 0, x11rb::NONE),
                    Some(rs) => conn.shape_rectangles(SO::SET, kind, ClipOrdering::UNSORTED, self.container, 0, 0, &rs),
                };
            }
            st.shape = Some(shape);
        }
        if !st.mapped {
            let _ = conn.map_window(self.container);
            st.mapped = true;
        }
        drop(st);
        self.x.flush();
        resized
    }

    /// The page is not drawn this frame: out of sight, its place kept.
    pub(crate) fn park(&self) {
        let mut st = self.st.borrow_mut();
        if std::mem::take(&mut st.mapped) {
            let _ = self.x.conn.unmap_window(self.container);
            self.x.flush();
        }
    }

    /// The keyboard goes to the page, or with `false` back to the GPUI window it is in.
    pub(crate) fn focus(&self, focused: bool) {
        let st = self.st.borrow();
        let target = if focused { self.browser_window() } else { Some(st.parent).filter(|w| *w != self.x.root) };
        if let (true, Some(target)) = (st.mapped || !focused, target) {
            let _ = self.x.conn.set_input_focus(InputFocus::PARENT, target, x11rb::CURRENT_TIME);
            self.x.flush();
        }
    }

    /// What the page shows now, as GPUI's BGRA, for the holes overlays cut into it.
    pub(crate) fn snapshot(&self) -> Option<(Vec<u8>, u32, u32)> {
        let frame = self.st.borrow().frame;
        let (w, h) = (u16::try_from(frame.w).ok()?, u16::try_from(frame.h).ok()?);
        let source = self.browser_window().unwrap_or(self.container);
        let reply = self.x.conn.get_image(ImageFormat::Z_PIXMAP, source, 0, 0, w, h, !0).ok()?.reply().ok()?;
        let mut data = reply.data;
        if data.len() != usize::from(w) * usize::from(h) * 4 {
            return None;
        }
        for pixel in data.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        Some((data, u32::from(w), u32::from(h)))
    }

    /// The page closed: the container leaves the window for the root, unmapped, until
    /// Chromium's browser in it is gone ([`released`]).
    pub(crate) fn teardown(&self) {
        self.park();
        let _ = self.x.conn.reparent_window(self.container, self.x.root, 0, 0);
        self.x.flush();
        RELEASED.with(|r| r.borrow_mut().insert(self.page, self.container));
    }
}

/// Page `id`'s browser is gone: its container goes too.
pub(crate) fn released(id: u64) {
    let Some(container) = RELEASED.with(|r| r.borrow_mut().remove(&id)) else { return };
    LIVE.with(|l| l.set(l.get().saturating_sub(1)));
    if let Ok(x) = x() {
        let _ = x.conn.destroy_window(container);
        x.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(rs: &[R]) -> i32 {
        rs.iter().map(|r| r.w * r.h).sum()
    }

    #[test]
    fn a_hole_in_the_middle_leaves_four_pieces() {
        let out = subtract(&[R::new(0, 0, 100, 100)], R::new(40, 40, 20, 20));
        assert_eq!(out.len(), 4);
        assert_eq!(area(&out), 100 * 100 - 20 * 20);
        for (i, a) in out.iter().enumerate() {
            assert!(out[i + 1..].iter().all(|b| a.intersect(*b).is_none()), "pieces overlap");
            assert!(a.intersect(R::new(40, 40, 20, 20)).is_none(), "a piece covers the hole");
        }
    }

    #[test]
    fn a_hole_over_everything_leaves_nothing_and_one_outside_changes_nothing() {
        assert!(subtract(&[R::new(10, 10, 50, 50)], R::new(0, 0, 100, 100)).is_empty());
        let page = [R::new(10, 10, 50, 50)];
        assert_eq!(subtract(&page, R::new(70, 70, 10, 10)), page);
    }

    #[test]
    fn a_modal_backdrop_takes_all_input_and_cuts_nothing() {
        let frame = R::new(0, 40, 800, 560);
        let s = shape(frame, Some(frame), &[(R::new(0, 0, 800, 600), false, true)], 1.);
        assert_eq!(s.bounding, vec![R::new(0, 0, 800, 560)]);
        assert!(s.input.is_empty());
        assert!(!s.cut);
    }

    #[test]
    fn a_popover_cuts_a_padded_hole_in_window_coordinates() {
        let frame = R::new(100, 100, 400, 300);
        let popover = R::new(150, 120, 100, 50);
        let s = shape(frame, Some(frame), &[(popover, true, true)], 2.);
        assert!(s.cut);
        let hole = popover.inflate(24).offset(-100, -100);
        assert!(s.bounding.iter().all(|r| r.intersect(hole).is_none()));
        assert_eq!(area(&s.bounding), 400 * 300 - area(&[hole.intersect(R::new(0, 0, 400, 300)).unwrap()]));
    }

    #[test]
    fn a_page_scrolled_out_of_its_clip_shows_nothing() {
        let frame = R::new(0, 0, 100, 100);
        assert_eq!(shape(frame, Some(R::new(200, 0, 10, 10)), &[], 1.), Shape::default());
        let s = shape(frame, Some(R::new(0, 50, 100, 100)), &[], 1.);
        assert_eq!(s.bounding, vec![R::new(0, 50, 100, 50)]);
    }
}
