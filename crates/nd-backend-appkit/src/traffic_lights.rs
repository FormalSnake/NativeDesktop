//! Traffic lights at window coordinates (`window_controls().at(..)`, ARCHITECTURE.md 5.4), a
//! port of the old `NDShell/WindowControls.swift`. The buttons stay the window's own (hover
//! group, menu, accessibility); only their frames move. AppKit puts them back whenever it lays
//! out the title bar (resize, key change, full screen), so placement is re-asserted from those
//! moments and from the buttons' own frame changes, never done once.

use std::cell::{Cell, RefCell};

use nd_core::Point;
use objc2::rc::Retained;
use objc2_app_kit::{NSButton, NSView, NSWindow, NSWindowButton, NSWindowStyleMask, NSWindowTitleVisibility};
use objc2_foundation::{NSPoint, NSRect};

const KINDS: [NSWindowButton; 3] =
    [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton];

/// AppKit's own layout, read before anything moved.
struct Stock {
    container: NSRect,
    buttons: [NSRect; 3],
}

pub(crate) struct TrafficLights {
    /// Top-left of the close button, in points from the window's top-left corner.
    at: Cell<Option<Point>>,
    stock: RefCell<Option<Stock>>,
    applying: Cell<bool>,
    diag: bool,
    reported: Cell<Option<Point>>,
}

pub(crate) fn buttons(window: &NSWindow) -> Option<[Retained<NSButton>; 3]> {
    let [a, b, c] = KINDS.map(|k| window.standardWindowButton(k));
    Some([a?, b?, c?])
}

impl TrafficLights {
    pub fn new(diag: bool) -> Self {
        Self {
            at: Cell::new(None),
            stock: RefCell::new(None),
            applying: Cell::new(false),
            diag,
            reported: Cell::new(None),
        }
    }

    /// From the window's `window_controls()` node. Pinned lights mean the app draws its own
    /// title row, so the stock title and title bar material would sit beside the moved buttons.
    pub fn set_at(&self, window: &NSWindow, at: Option<Point>) {
        if self.at.replace(at) == at {
            return;
        }
        let pinned = at.is_some();
        window.setTitleVisibility(if pinned {
            NSWindowTitleVisibility::Hidden
        } else {
            NSWindowTitleVisibility::Visible
        });
        window.setTitlebarAppearsTransparent(pinned);
        if !pinned && let (Some(stock), Some(buttons)) = (self.stock.borrow().as_ref(), buttons(window)) {
            if let Some(container) = unsafe { buttons[0].superview().and_then(|s| s.superview()) } {
                let mut frame = container.frame();
                frame.size.height = stock.container.size.height;
                frame.origin.y = window.frame().size.height - stock.container.size.height;
                container.setFrame(frame);
            }
            for (button, frame) in buttons.iter().zip(&stock.buttons) {
                button.setFrameOrigin(frame.origin);
            }
        }
        self.apply(window);
    }

    fn stock(&self, buttons: &[Retained<NSButton>; 3], container: &NSView) -> bool {
        if self.stock.borrow().is_some() {
            return true;
        }
        let frames = buttons.each_ref().map(|b| b.frame());
        // Read while AppKit is halfway through putting the buttons back, they overlap, and every
        // placement after would copy that. Not kept then; the next pass reads again.
        let steps_ok = frames.windows(2).all(|w| (16.0..=32.0).contains(&(w[1].origin.x - w[0].origin.x)));
        if steps_ok {
            *self.stock.borrow_mut() = Some(Stock { container: container.frame(), buttons: frames });
        }
        steps_ok
    }

    /// Re-asserts the placement. Returns the close button's top-left in window coordinates from
    /// the top-left when the buttons sit where configured.
    pub fn apply(&self, window: &NSWindow) -> Option<Point> {
        let at = self.at.get()?;
        if self.applying.replace(true) {
            return None;
        }
        let placed = self.place(window, at);
        self.applying.set(false);
        if self.diag
            && let Some(p) = placed
            && placed != self.reported.get()
        {
            eprintln!("nd: traffic lights close={},{}", p.x, p.y);
            self.reported.set(placed);
        }
        placed
    }

    fn place(&self, window: &NSWindow, at: Point) -> Option<Point> {
        let buttons = buttons(window)?;
        let container = unsafe { buttons[0].superview()?.superview()? };
        if !self.stock(&buttons, &container) {
            return None;
        }
        // Full screen draws the title bar in a reveal window AppKit owns.
        if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
            return None;
        }
        let stock = self.stock.borrow();
        let stock = stock.as_ref()?;
        let window_height = window.frame().size.height;
        let height = stock.buttons[0].size.height;
        let centre_from_top = f64::from(at.y) + height / 2.;
        // The band the buttons sit in has to reach twice their centre, so they stay vertically
        // centred in it the way AppKit centres them in a toolbar band.
        let band = stock.container.size.height.max((centre_from_top * 2.).round());
        let mut frame = container.frame();
        frame.size.height = band;
        frame.origin.y = window_height - band;
        if frame != container.frame() {
            container.setFrame(frame);
        }
        let first = stock.buttons[0].origin.x;
        for (button, stock_frame) in buttons.iter().zip(&stock.buttons) {
            let x = (f64::from(at.x) + stock_frame.origin.x - first).round();
            let top = (centre_from_top - stock_frame.size.height / 2.).round();
            // The title bar view's y axis runs either way depending on the OS release.
            let flipped = unsafe { button.superview() }.is_some_and(|s| s.isFlipped());
            let y = if flipped { top } else { band - top - stock_frame.size.height };
            let origin = NSPoint::new(x, y);
            if button.frame().origin != origin {
                button.setFrameOrigin(origin);
            }
        }
        let close = buttons[0].convertRect_toView(buttons[0].bounds(), None);
        Some(Point { x: close.origin.x as f32, y: (window_height - close.origin.y - close.size.height) as f32 })
    }
}
