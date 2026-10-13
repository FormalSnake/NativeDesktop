//! The custom title row (`titlebar()`, ARCHITECTURE.md 5.4): its empty area drags the window
//! (`performWindowDragWithEvent`) and a double-click does what the user chose in System
//! Settings; its insets are measured from the window's own buttons and title bar, never
//! constants.

use nd_components::{InsetEdge, TitlebarInsetProps};
use nd_core::Size;
use objc2_app_kit::{NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSString, NSUserDefaults};

use crate::traffic_lights;

/// `AppleActionOnDoubleClick`: zoom (the default, "Maximize" or "Fill"), minimize, or nothing.
pub(crate) fn double_click(window: &NSWindow) {
    let action = NSUserDefaults::standardUserDefaults().stringForKey(&NSString::from_str("AppleActionOnDoubleClick"));
    match action.map(|a| a.to_string()).as_deref() {
        Some("Minimize") => window.performMiniaturize(None),
        Some("None") => {}
        _ => window.performZoom(None),
    }
}

/// The OS title bar's height: the window minus its content layout rect, or the buttons'
/// band when the window has no title bar strip (full screen).
fn titlebar_height(window: &NSWindow) -> f32 {
    let height = window.frame().size.height - window.contentLayoutRect().size.height;
    if height > 0. {
        return height as f32;
    }
    traffic_lights::buttons(window).map_or(0., |b| b[0].frame().size.height as f32)
}

/// The size a title row inset takes: on the leading edge, the span from the window's left
/// edge past the traffic lights plus the gap AppKit leaves between them; nothing trailing.
pub(crate) fn measure_inset(window: &NSWindow, props: &TitlebarInsetProps) -> Size {
    let height = titlebar_height(window);
    if props.edge == InsetEdge::Trailing || props.controls_in_row {
        return Size { width: 0., height };
    }
    if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
        return Size { width: 0., height };
    }
    let Some(buttons) = traffic_lights::buttons(window) else { return Size { width: 0., height } };
    let frames = buttons.each_ref().map(|b| b.convertRect_toView(b.bounds(), None));
    let gap = (frames[1].origin.x - (frames[0].origin.x + frames[0].size.width)).max(0.);
    let right = frames[2].origin.x + frames[2].size.width;
    // The lights' own leading margin, mirrored after them.
    let lead = frames[0].origin.x.max(gap);
    Size { width: (right + lead) as f32, height: height.max((frames[0].size.height + 2. * gap) as f32) }
}
