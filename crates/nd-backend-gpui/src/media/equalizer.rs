//! `equalizer` on GPUI: the four bars as rounded quads, their heights from the shared wave at the
//! time of the frame, asking for the next frame while they move. Paused, or under Reduce Motion,
//! they hold `EQUALIZER.still` and ask for nothing.

use std::sync::OnceLock;

use nd_components::{EQUALIZER, EqualizerProps, equalizer_bars, equalizer_level};
use nd_core::time::Instant;
use nd_gpui::gpui::{AnyElement, IntoElement, ParentElement, Styled, Window, div, px};

use crate::render::Build;

/// One clock for every equalizer, so two on screen move alike.
fn seconds() -> f32 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}

pub(super) fn equalizer(b: &Build, p: &EqualizerProps, window: &mut Window) -> AnyElement {
    let color = p.style.text.color.unwrap_or(b.theme().accent);
    let moving = p.playing && b.backend.platform_reduce_motion() != Some(true);
    let t = seconds();
    if moving {
        window.request_animation_frame();
    }
    let bars = equalizer_bars(p.size, |i| if moving { equalizer_level(i, t) } else { EQUALIZER.still[i] });
    div()
        .relative()
        .size(px(p.size))
        .flex_none()
        .children(bars.into_iter().map(|(x, y, w, h)| {
            div().absolute().left(px(x)).top(px(y)).w(px(w)).h(px(h)).rounded(px(w / 2.)).bg(color)
        }))
        .into_any_element()
}
