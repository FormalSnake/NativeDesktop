//! `bottom_accessory`: content with a bar along its foot, such as a music player's now playing.
//! macOS 26 and later float the bar on a Liquid Glass capsule inset from the edges, with the
//! content scrolling under it, as Music and Podcasts do. GNOME and WinUI dock it below a
//! separator.

use nd_ui::prelude::*;

use crate::controls::separator;
use crate::glass::glass;
use crate::media::PILL;

/// Space between the floating capsule and the content's sides.
const SIDE_INSET: f32 = 40.;
/// Space between the floating capsule and the content's foot.
const FOOT_INSET: f32 = 12.;

/// What scrolling content leaves free at its end so its last rows can rise above a floating
/// bar. Zero where the bar docks.
pub fn bottom_accessory_clearance() -> f32 {
    if cfg!(target_os = "macos") { PILL.height + 2. * FOOT_INSET } else { 0. }
}

pub struct BottomAccessory {
    id: SharedString,
    content: Option<AnyElement>,
    bar: Option<AnyElement>,
}

pub fn bottom_accessory(id: impl Into<SharedString>) -> BottomAccessory {
    BottomAccessory { id: id.into(), content: None, bar: None }
}

impl BottomAccessory {
    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }

    /// The bar; none shows the content alone.
    pub fn bar(mut self, bar: impl IntoElement) -> Self {
        self.bar = Some(bar.into_any_element());
        self
    }
}

impl IntoElement for BottomAccessory {
    fn into_any_element(self) -> AnyElement {
        let BottomAccessory { id, content, bar } = self;
        let content = div().flex_1().min_h_0().w_full().children(content);
        if cfg!(target_os = "macos") {
            div()
                .id(id.clone())
                .relative()
                .flex()
                .flex_col()
                .size_full()
                .child(content)
                .when_some(bar, |root, bar| {
                    root.child(
                        div().absolute().left(px(SIDE_INSET)).right(px(SIDE_INSET)).bottom(px(FOOT_INSET)).child(
                            glass()
                                .id(format!("{id}.glass"))
                                .h(px(PILL.height))
                                .rounded(px(PILL.height / 2.))
                                .px_3()
                                .justify_center()
                                .child(bar),
                        ),
                    )
                })
                .into_any_element()
        } else {
            v_flex()
                .id(id)
                .size_full()
                .child(content)
                .when_some(bar, |root, bar| root.child(separator()).child(bar))
                .into_any_element()
        }
    }
}
