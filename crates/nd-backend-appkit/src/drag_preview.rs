//! The picture of an nd element dragged outside its window: a borderless panel holding a
//! snapshot of the element's view, centred under the pointer, ignoring the mouse so the windows
//! under it keep reporting where the pointer is. Inside its window the element draws itself.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::{AllocAnyThread, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSImage, NSImageView, NSPanel, NSScreen, NSView, NSWindowLevel, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};

thread_local! {
    static PANEL: RefCell<Option<Retained<NSPanel>>> = const { RefCell::new(None) };
}

/// Shows `view` under the pointer at `screen` (nd global points), or hides the picture.
pub(crate) fn show(mtm: MainThreadMarker, view: Option<&NSView>, screen: Option<nd_core::Point>) {
    let Some(screen) = screen else {
        PANEL.with(|p| {
            if let Some(panel) = p.borrow_mut().take() {
                panel.orderOut(None);
            }
        });
        return;
    };
    let existing = PANEL.with(|p| p.borrow().clone());
    let panel = match existing {
        Some(panel) => panel,
        None => {
            let Some(view) = view else { return };
            let Some(panel) = make(mtm, view) else { return };
            PANEL.with(|p| *p.borrow_mut() = Some(panel.clone()));
            panel
        }
    };
    let size = panel.frame().size;
    let primary = NSScreen::screens(mtm).firstObject().map_or(0., |s| s.frame().size.height);
    let origin = NSPoint::new(f64::from(screen.x) - size.width / 2., primary - f64::from(screen.y) - size.height / 2.);
    panel.setFrameOrigin(origin);
    panel.orderFrontRegardless();
}

fn make(mtm: MainThreadMarker, view: &NSView) -> Option<Retained<NSPanel>> {
    let bounds = view.bounds();
    if bounds.size.width < 1. || bounds.size.height < 1. {
        return None;
    }
    let rep = view.bitmapImageRepForCachingDisplayInRect(bounds)?;
    view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
    let image = NSImage::initWithSize(NSImage::alloc(), bounds.size);
    image.addRepresentation(&rep);
    let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(bounds.size.width, bounds.size.height));
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        frame,
        NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
        NSBackingStoreType::Buffered,
        false,
    );
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(true);
    panel.setIgnoresMouseEvents(true);
    panel.setAlphaValue(0.9);
    panel.setLevel(NSWindowLevel::from(objc2_app_kit::NSPopUpMenuWindowLevel));
    unsafe { panel.setReleasedWhenClosed(false) };
    let picture = NSImageView::imageViewWithImage(&image, mtm);
    picture.setFrame(frame);
    panel.setContentView(Some(&picture));
    Some(panel)
}
