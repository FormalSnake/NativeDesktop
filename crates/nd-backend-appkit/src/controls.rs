//! `separator` as `NSBox` of the separator type. The form controls live in `crate::forms`.

use objc2::rc::Retained;
use objc2_app_kit::{NSBox, NSBoxType};
use objc2_foundation::MainThreadMarker;

/// `NSBox` of the separator type.
pub(crate) fn separator(mtm: MainThreadMarker) -> Retained<NSBox> {
    let b = NSBox::new(mtm);
    b.setBoxType(NSBoxType::Separator);
    b
}
