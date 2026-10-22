//! An expanded view presented over a region: a now-playing screen over the page, a detail over
//! its list. Inside it a `.hero_with(..)` element (the artwork) travels from its source and back,
//! so the motion says where the view came from, while the backdrop fades in under it on the same
//! spring. The rest of the view marks itself `.reveal_with(&state)`: it fades in once the hero
//! has landed and out before the hero leaves, so nothing draws over the hero in flight. The base
//! stays mounted, keeping its scroll and state, and is hidden once covered.
//!
//! ```ignore
//! // The view keeps one handle; open and close it, then render.
//! self.expanded.set_open(true);
//! presentation("now-playing", &self.expanded)
//!     .base(page)
//!     .backdrop(image(cover).blur(px(60.)))
//!     .content(self.now_playing.clone())
//!     .bg(cx.theme().surface.base)
//! // Inside the content, the artwork, and the rest revealed with it:
//! div().size(px(480.)).child(
//!     div().absolute().size(px(480.)).hero_with(&expanded, "player.bar.art").child(art),
//! )
//! v_flex().reveal_with(&expanded).child(title)
//! ```
//!
//! Interruptible: closing mid-open turns everything around from where it is. Reduce motion
//! leaves short fades and no travel.

use std::cell::Cell;
use std::rc::Rc;

use nd_gpui::gpui::Refineable;
use nd_ui::prelude::*;
use nd_ui::{Pace, StyleRefinement};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Closed,
    Opening,
    Open,
    Closing,
}

/// Whether a presentation is open, kept by the view that shows it. Cheap to clone.
#[derive(Clone, Debug, Default)]
pub struct Presentation(Rc<State>);

#[derive(Debug, Default)]
struct State {
    phase: Cell<Phase>,
    /// While closing: the backdrop has faded out, and the hero (if one rendered) has landed.
    faded: Cell<bool>,
    landed: Cell<bool>,
    hero: Cell<bool>,
}

impl Presentation {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens or closes it; the view renders again to show the change.
    pub fn set_open(&self, open: bool) {
        let next = match (self.0.phase.get(), open) {
            (Phase::Closed | Phase::Closing, true) => Phase::Opening,
            (Phase::Opening | Phase::Open, false) => {
                self.0.faded.set(false);
                self.0.landed.set(false);
                Phase::Closing
            }
            (phase, _) => phase,
        };
        self.0.phase.set(next);
    }

    /// Open, or opening.
    pub fn is_open(&self) -> bool {
        matches!(self.0.phase.get(), Phase::Opening | Phase::Open)
    }

    /// Going away: a hero inside travels back to its source.
    pub fn is_closing(&self) -> bool {
        self.0.phase.get() == Phase::Closing
    }

    /// On screen, coming, staying or going.
    pub fn is_shown(&self) -> bool {
        self.0.phase.get() != Phase::Closed
    }

    /// The backdrop's fade ended. Closing ends once the hero has landed too, so the content
    /// stays mounted under it the whole way back.
    fn faded(&self) {
        match self.0.phase.get() {
            Phase::Opening => self.0.phase.set(Phase::Open),
            Phase::Closing => {
                self.0.faded.set(true);
                self.close_if_done();
            }
            _ => {}
        }
    }

    fn landed(&self) {
        if self.0.phase.get() == Phase::Closing {
            self.0.landed.set(true);
            self.close_if_done();
        }
    }

    fn close_if_done(&self) {
        if self.0.faded.get() && (self.0.landed.get() || !self.0.hero.get()) {
            self.0.phase.set(Phase::Closed);
        }
    }
}

pub struct PresentationView {
    id: SharedString,
    state: Presentation,
    base: Option<AnyElement>,
    backdrop: Option<AnyElement>,
    content: Option<AnyElement>,
    layer: StyleRefinement,
}

/// `content` presented over `base` while `state` is open. Style it with the backdrop's
/// background.
#[track_caller]
pub fn presentation(id: impl Into<SharedString>, state: &Presentation) -> PresentationView {
    PresentationView {
        id: id.into(),
        state: state.clone(),
        base: None,
        backdrop: None,
        content: None,
        layer: StyleRefinement::default(),
    }
}

impl PresentationView {
    /// What sits under the presentation, mounted throughout.
    pub fn base(mut self, base: impl IntoElement) -> Self {
        self.base = Some(base.into_any_element());
        self
    }

    /// Drawn over the background and under the content, fading with it: blurred artwork, say.
    /// It fills the region and is clipped to it.
    pub fn backdrop(mut self, backdrop: impl IntoElement) -> Self {
        self.backdrop = Some(backdrop.into_any_element());
        self
    }

    /// The presented view, mounted only while shown.
    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }
}

impl Styled for PresentationView {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.layer
    }
}

impl IntoElement for PresentationView {
    fn into_any_element(self) -> AnyElement {
        let PresentationView { id, state, base, backdrop, content, layer } = self;
        let covered = state.0.phase.get() == Phase::Open;
        let shown = state.is_shown();
        let target = if state.is_open() { 1. } else { 0. };
        // Comes in on the hero's spring, goes out with the hero once the content has faded.
        let pace = if state.is_closing() { Animation::Themed(Pace::Exit) } else { Animation::natural() };
        let mut fill = div()
            .id(id)
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .overflow_hidden()
            .opacity(target)
            .animate(pace)
            .enter_from(|p| p.opacity(0.))
            .on_animation_end(move |_, _, cx| {
                state.faded();
                cx.refresh_windows();
            })
            .children(backdrop);
        fill.style().refine(&layer);
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .child(div().size_full().when(covered, |b| b.invisible()).children(base))
            .when(shown, |root| root.child(fill).child(div().absolute().top_0().left_0().size_full().children(content)))
            .into_any_element()
    }
}

pub trait PresentationExt: Styled + Sized {
    /// Fades in once a presentation's hero has landed and out before it leaves, so it never
    /// draws over the hero in flight.
    fn reveal_with(self, state: &Presentation) -> Self;

    /// The presentation's shared element: it travels from the element keyed `from` as the
    /// presentation opens and back onto it as it closes ([`Div::hero_from`]), and the
    /// presentation stays on screen until it has landed.
    fn hero_with(self, state: &Presentation, from: impl Into<SharedString>) -> Self;
}

impl PresentationExt for Div {
    fn reveal_with(self, state: &Presentation) -> Self {
        let pace = if state.is_open() { Animation::Themed(Pace::Settled) } else { Animation::fast() };
        self.opacity(if state.is_open() { 1. } else { 0. }).animate(pace).enter_from(|p| p.opacity(0.))
    }

    fn hero_with(self, state: &Presentation, from: impl Into<SharedString>) -> Self {
        state.0.hero.set(true);
        let state = state.clone();
        self.hero_from(from, state.is_closing()).on_animation_end(move |_, _, cx| {
            state.landed();
            cx.refresh_windows();
        })
    }
}
