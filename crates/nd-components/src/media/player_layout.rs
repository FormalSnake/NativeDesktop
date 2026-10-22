//! The expanded player's layout, as Music lays it out at each size. Wide, the artwork and the
//! song's details sit in a column on the left and a panel (lyrics, the queue) fills the right.
//! Narrow, a header row holds a small artwork beside the details, and the panel takes the room
//! under it. The artwork is sized from the room the layout measured, in points, so it never
//! leaves its column, never runs under the text, and can travel as a hero.
//!
//! ```ignore
//! player_layout("np", &self.layout)
//!     .artwork(|side| cover.size(px(side)))
//!     .details(v_flex().child(label(title)).child(label(artist)))
//!     .panel(lyrics)
//! ```

use std::cell::Cell;
use std::rc::Rc;

use nd_ui::Resized;
use nd_ui::prelude::*;

/// Below this width the artwork goes into a header row above the panel.
pub const WIDE_FROM: f32 = 760.;
/// The artwork's side in the header row.
pub const HEADER_ART: f32 = 64.;
const MAX_ART: f32 = 480.;
const MIN_ART: f32 = 120.;
const PANEL: (f32, f32) = (300., 440.);
const PAD: f32 = 24.;
const GAP: f32 = 32.;
/// Room under the artwork for the details (title, artist, a switch).
const DETAILS: f32 = 140.;

/// The room the layout was given, kept by the view across renders. Cheap to clone.
#[derive(Clone, Debug, Default)]
pub struct PlayerLayoutState(Rc<Cell<(f32, f32)>>);

impl PlayerLayoutState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Wide enough for the artwork column beside the panel; before the first layout, wide.
    pub fn is_wide(&self) -> bool {
        let (w, _) = self.0.get();
        w == 0. || w >= WIDE_FROM
    }

    /// The artwork's side at the measured size.
    pub fn art_side(&self) -> f32 {
        let (w, h) = self.0.get();
        if !self.is_wide() {
            return HEADER_ART;
        }
        if w == 0. {
            return MAX_ART;
        }
        let column = w - 2. * PAD - GAP - panel_width(w);
        column.min(h - 2. * PAD - DETAILS).clamp(MIN_ART, MAX_ART).floor()
    }
}

fn panel_width(width: f32) -> f32 {
    (width * 0.4).clamp(PANEL.0, PANEL.1)
}

type Artwork = Box<dyn FnOnce(f32) -> AnyElement>;

pub struct PlayerLayout {
    id: SharedString,
    state: PlayerLayoutState,
    artwork: Option<Artwork>,
    details: Option<AnyElement>,
    panel: Option<AnyElement>,
}

/// The expanded player's layout, adapting to the room it is given.
#[track_caller]
pub fn player_layout(id: impl Into<SharedString>, state: &PlayerLayoutState) -> PlayerLayout {
    PlayerLayout { id: id.into(), state: state.clone(), artwork: None, details: None, panel: None }
}

impl PlayerLayout {
    /// The artwork at `side` points square.
    pub fn artwork<E: IntoElement>(mut self, build: impl FnOnce(f32) -> E + 'static) -> Self {
        self.artwork = Some(Box::new(move |side| build(side).into_any_element()));
        self
    }

    /// The song's details: under the artwork when wide, beside it when narrow.
    pub fn details(mut self, details: impl IntoElement) -> Self {
        self.details = Some(details.into_any_element());
        self
    }

    /// Lyrics, the queue: right of the artwork when wide, under the header when narrow.
    pub fn panel(mut self, panel: impl IntoElement) -> Self {
        self.panel = Some(panel.into_any_element());
        self
    }
}

impl IntoElement for PlayerLayout {
    fn into_any_element(self) -> AnyElement {
        let PlayerLayout { id, state, artwork, details, panel } = self;
        let side = state.art_side();
        let art = artwork.map(|build| div().flex_none().size(px(side)).child(build(side)));
        let measured = state.clone();
        let root = div().id(id).size_full().p(px(PAD)).overflow_hidden().on_resize(move |r: &Resized, _, cx| {
            let before = (measured.is_wide(), measured.art_side());
            measured.0.set((r.width, r.height));
            if (measured.is_wide(), measured.art_side()) != before {
                cx.refresh_windows();
            }
        });
        if state.is_wide() {
            let width = state.0.get().0;
            let panel_w = if width == 0. { PANEL.1 } else { panel_width(width) };
            root.flex()
                .flex_row()
                .gap(px(GAP))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .items_center()
                        .justify_center()
                        .gap_4()
                        .children(art)
                        .child(div().w_full().min_w_0().flex().flex_col().items_center().children(details)),
                )
                .child(v_flex().w(px(panel_w)).h_full().flex_none().min_h_0().children(panel))
                .into_any_element()
        } else {
            root.flex()
                .flex_col()
                .gap_4()
                .child(
                    h_flex()
                        .w_full()
                        .flex_none()
                        .items_center()
                        .gap_4()
                        .children(art)
                        .child(div().flex_1().min_w_0().children(details)),
                )
                .child(v_flex().w_full().flex_1().min_h_0().children(panel))
                .into_any_element()
        }
    }
}
