//! Quick Look: the system's `QLPreviewPanel`, the panel Finder opens with Space, over a list of
//! files. The panel asks a data source for the items and a delegate for the frame each one
//! zooms from; both are one object here, kept alive while the panel can ask.

use std::cell::RefCell;
use std::path::PathBuf;

use nd_core::Rect;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::NSWindow;
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString, NSURL};

/// `QLPreviewPanel`, its framework loaded on first use: nothing links against it statically, so
/// the linker would drop a bare `#[link]`.
fn panel_class() -> Option<&'static objc2::runtime::AnyClass> {
    if let Some(class) = objc2::runtime::AnyClass::get(c"QLPreviewPanel") {
        return Some(class);
    }
    let path = NSString::from_str("/System/Library/Frameworks/QuickLookUI.framework");
    let bundle = objc2_foundation::NSBundle::bundleWithPath(&path)?;
    // SAFETY: a system framework, loaded once on the main thread.
    if !unsafe { bundle.load() } {
        return None;
    }
    objc2::runtime::AnyClass::get(c"QLPreviewPanel")
}

#[derive(Default)]
pub struct SourceIvars {
    items: RefCell<Vec<(Retained<NSURL>, Option<NSRect>)>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; the methods are the panel's data source
    // and delegate callbacks, called on the main thread.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = SourceIvars]
    struct NdQuickLookSource;

    impl NdQuickLookSource {
        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn count(&self, _panel: *mut AnyObject) -> isize {
            self.ivars().items.borrow().len() as isize
        }

        #[unsafe(method_id(previewPanel:previewItemAtIndex:))]
        fn item(&self, _panel: *mut AnyObject, index: isize) -> Option<Retained<NSURL>> {
            let index = usize::try_from(index).ok();
            index.and_then(|i| self.ivars().items.borrow().get(i).map(|(url, _)| url.clone()))
        }

        #[unsafe(method(previewPanel:sourceFrameOnScreenForPreviewItem:))]
        fn source_frame(&self, _panel: *mut AnyObject, item: *mut AnyObject) -> NSRect {
            let items = self.ivars().items.borrow();
            items
                .iter()
                .find(|(url, _)| std::ptr::eq(Retained::as_ptr(url).cast::<AnyObject>(), item))
                .and_then(|(_, frame)| *frame)
                .unwrap_or(NSRect::new(NSPoint::ZERO, NSSize::ZERO))
        }

        #[unsafe(method(previewPanel:handleEvent:))]
        fn handle_event(&self, _panel: *mut AnyObject, _event: *mut AnyObject) -> Bool {
            Bool::NO
        }
    }
);

thread_local! {
    static SOURCE: RefCell<Option<Retained<NdQuickLookSource>>> = const { RefCell::new(None) };
}

/// A screen rect for `bounds` (top-left window points, as nd lays out) in `window`.
pub fn screen_rect(window: &NSWindow, bounds: Rect) -> Option<NSRect> {
    let content = window.contentView()?;
    let height = content.frame().size.height;
    let local = NSRect::new(
        NSPoint::new(f64::from(bounds.origin.x), height - f64::from(bounds.origin.y + bounds.size.height)),
        NSSize::new(f64::from(bounds.size.width), f64::from(bounds.size.height)),
    );
    let in_window = content.convertRect_toView(local, None);
    Some(window.convertRectToScreen(in_window))
}

/// Opens Quick Look on `items` at `index`, each zooming from its screen frame when it has one.
/// While it is open, Space and Esc close it and the arrow keys step through the items.
pub fn show(items: Vec<(PathBuf, Option<NSRect>)>, index: usize) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let source = SOURCE.with(|s| {
        s.borrow_mut()
            .get_or_insert_with(|| {
                let this = NdQuickLookSource::alloc(mtm).set_ivars(SourceIvars::default());
                unsafe { msg_send![super(this), init] }
            })
            .clone()
    });
    *source.ivars().items.borrow_mut() = items
        .into_iter()
        .map(|(path, frame)| (NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy())), frame))
        .collect();
    let Some(class) = panel_class() else { return };
    // SAFETY: QLPreviewPanel's documented class method and properties; the source outlives the
    // panel's use of it (it lives in SOURCE until the next call replaces its items).
    unsafe {
        let panel: Option<Retained<AnyObject>> = msg_send![class, sharedPreviewPanel];
        let Some(panel) = panel else { return };
        let _: () = msg_send![&*panel, setDataSource: &*source];
        let _: () = msg_send![&*panel, setDelegate: &*source];
        let _: () = msg_send![&*panel, reloadData];
        let _: () = msg_send![&*panel, setCurrentPreviewItemIndex: index as isize];
        let _: () = msg_send![&*panel, makeKeyAndOrderFront: Option::<&AnyObject>::None];
    }
}

/// Whether the Quick Look panel is on screen.
pub fn is_open() -> bool {
    let Some(class) = objc2::runtime::AnyClass::get(c"QLPreviewPanel") else { return false };
    unsafe {
        let exists: Bool = msg_send![class, sharedPreviewPanelExists];
        if !exists.as_bool() {
            return false;
        }
        let panel: Retained<AnyObject> = msg_send![class, sharedPreviewPanel];
        let visible: Bool = msg_send![&*panel, isVisible];
        visible.as_bool()
    }
}

/// Closes the panel when it is open.
pub fn close() {
    if !is_open() {
        return;
    }
    let Some(class) = objc2::runtime::AnyClass::get(c"QLPreviewPanel") else { return };
    unsafe {
        let panel: Retained<AnyObject> = msg_send![class, sharedPreviewPanel];
        let _: () = msg_send![&*panel, orderOut: Option::<&AnyObject>::None];
    }
}
