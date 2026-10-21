//! A control's fill easing between its rest, hover and pressed colours, as each look times it:
//! WinUI fades a control's state linearly in 83 ms (Fluent's "bare minimum" fade), libadwaita
//! over 200 ms on its ease-out-quad, the other looks over their preset's fast motion. With the
//! OS's animations off the colour changes at once.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use nd_core::NodeId;
use nd_gpui::gpui::Hsla;
use nd_theme::color::mix;

/// Where the pointer is on a control.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Pointer {
    pub hovered: bool,
    pub pressed: bool,
}

#[derive(Clone, Copy, Debug)]
struct Fade {
    from: Hsla,
    to: Hsla,
    start: f64,
    pointer: Pointer,
}

/// Every faded control's state, by node.
#[derive(Clone, Default)]
pub(crate) struct Fades(Rc<RefCell<HashMap<NodeId, Fade>>>);

impl Fades {
    pub fn pointer(&self, id: NodeId) -> Pointer {
        self.0.borrow().get(&id).map(|f| f.pointer).unwrap_or_default()
    }

    pub fn set_pointer(&self, id: NodeId, f: impl FnOnce(&mut Pointer)) {
        if let Some(fade) = self.0.borrow_mut().get_mut(&id) {
            f(&mut fade.pointer);
        }
    }

    pub fn forget(&self, id: NodeId) {
        self.0.borrow_mut().remove(&id);
    }

    /// The colour `id` shows at `now` while heading for `target` over `duration` seconds, and
    /// whether it is still on its way.
    /// `curve` is a cubic-bezier's control points ([`LINEAR`] for a straight fade).
    pub fn sample(&self, id: NodeId, target: Hsla, now: f64, duration: f64, curve: [f32; 4]) -> (Hsla, bool) {
        let mut fades = self.0.borrow_mut();
        let fade =
            fades.entry(id).or_insert(Fade { from: target, to: target, start: now, pointer: Pointer::default() });
        if fade.to != target {
            // Retargeted mid-fade: start from what is on screen, so nothing jumps.
            let shown = at(fade, now, duration, curve);
            *fade = Fade { from: shown, to: target, start: now, pointer: fade.pointer };
        }
        let t = progress(fade.start, now, duration);
        (at(fade, now, duration, curve), t < 1. && fade.from != fade.to)
    }
}

fn progress(start: f64, now: f64, duration: f64) -> f64 {
    if duration <= 0. { 1. } else { ((now - start) / duration).clamp(0., 1.) }
}

/// A straight fade, as Fluent's opacity and colour fades are.
pub(crate) const LINEAR: [f32; 4] = [0., 0., 1., 1.];

fn at(fade: &Fade, now: f64, duration: f64, curve: [f32; 4]) -> Hsla {
    let t = progress(fade.start, now, duration) as f32;
    if t >= 1. { fade.to } else { mix(fade.from, fade.to, crate::forms::look::cubic_bezier(curve, t)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nd_gpui::gpui::{black, white};

    #[test]
    fn a_state_change_fades_over_the_duration_and_a_retarget_starts_from_what_shows() {
        let fades = Fades::default();
        let id = NodeId(1);
        assert_eq!(fades.sample(id, white(), 0., 0.083, LINEAR), (white(), false), "first sight is at rest");
        let (start, moving) = fades.sample(id, black(), 1., 0.083, LINEAR);
        assert_eq!(start, white(), "the fade starts where the colour was");
        assert!(moving);
        let (mid, _) = fades.sample(id, black(), 1.0415, 0.083, LINEAR);
        assert!(mid.l > 0.2 && mid.l < 0.8, "halfway: {mid:?}");
        let (back, _) = fades.sample(id, white(), 1.0415, 0.083, LINEAR);
        assert_eq!(back, mid, "heading back starts from the halfway colour");
        assert_eq!(fades.sample(id, white(), 2., 0.083, LINEAR), (white(), false));
        assert_eq!(fades.sample(id, black(), 3., 0., LINEAR), (black(), false), "no duration: at once");
    }
}
