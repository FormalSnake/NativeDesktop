//! The C ABI of the `NDSwift` package (`swift/NDSwift`). The archive is linked by the app's
//! build script (`nd::build::app()`), so these symbols stay undefined in this rlib.

use std::ffi::{CString, c_char, c_void};

use objc2::rc::Retained;
use objc2_app_kit::NSView;
use objc2_foundation::{MainThreadMarker, NSSize};

unsafe extern "C" {
    fn nd_swift_gauge_create(value: f64, lower: f64, upper: f64, label: *const c_char) -> *mut c_void;
    fn nd_swift_gauge_update(view: *mut c_void, value: f64, lower: f64, upper: f64, label: *const c_char);
    fn nd_swift_gauge_measure(view: *mut c_void, width: *mut f64, height: *mut f64);
}

/// A SwiftUI `Gauge` hosted in an `NSHostingView`.
pub struct SwiftGauge {
    view: Retained<NSView>,
}

impl SwiftGauge {
    pub fn new(_mtm: MainThreadMarker, value: f64, range: (f64, f64), label: &str) -> Self {
        let label = CString::new(label).expect("label has no NUL");
        // SAFETY: returns a +1 retained NSView (an NSHostingView), created on the main thread.
        let raw = unsafe { nd_swift_gauge_create(value, range.0, range.1, label.as_ptr()) };
        let view = unsafe { Retained::from_raw(raw.cast::<NSView>()) }.expect("swift returns a view");
        Self { view }
    }

    pub fn update(&self, value: f64, range: (f64, f64), label: &str) {
        let label = CString::new(label).expect("label has no NUL");
        unsafe { nd_swift_gauge_update(self.raw(), value, range.0, range.1, label.as_ptr()) }
    }

    pub fn measure(&self) -> NSSize {
        let (mut width, mut height) = (0.0, 0.0);
        unsafe { nd_swift_gauge_measure(self.raw(), &mut width, &mut height) };
        NSSize::new(width, height)
    }

    pub fn view(&self) -> &Retained<NSView> {
        &self.view
    }

    fn raw(&self) -> *mut c_void {
        Retained::as_ptr(&self.view).cast_mut().cast()
    }
}
