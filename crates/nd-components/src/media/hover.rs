//! Hover affordances on artwork, as Music draws them: while the pointer is over a card (or a
//! song row), its artwork darkens and Play and More buttons fade in over it. The card or row is
//! a `.group(..)`; [`artwork_actions`] sits over its artwork, in a box that clips to the
//! artwork's corners, and reveals itself with the group on the platform's hover timing. The
//! buttons stay in the accessibility tree at rest, so a screen reader reaches Play and More
//! without a pointer.
//!
//! The dim and the buttons are drawn over a picture, not over the window, so they are the same
//! in every scheme: black at a quarter over the artwork, white glyphs on a dark disc.

use std::rc::Rc;

use crate::controls::ButtonExt;
use nd_gpui::gpui::{black, white};
use nd_ui::Role;
use nd_ui::prelude::*;

use crate::icon::{Icon, icon};
use crate::media::equalizer::equalizer;
use crate::menu::ContextMenu;

/// What Play does.
pub type OnPlay = Rc<dyn Fn(&mut Window, &mut App)>;
/// Builds the More menu each time it opens.
pub type BuildMenu = Rc<dyn Fn(ContextMenu, &mut Window, &mut App) -> ContextMenu>;

/// Music's card buttons: 28 pt discs 8 pt in from the artwork's bottom corners.
const DISC: f32 = 28.;
const INSET: f32 = 8.;
const GLYPH: f32 = 13.;
/// A row's Play: the glyph alone, centred on its small artwork.
const ROW_GLYPH: f32 = 16.;
/// A row's now-playing bars: half its artwork, as YouTube Music draws them.
const ROW_BARS: f32 = 20.;
const DIM: f32 = 0.25;
const DISC_FILL: f32 = 0.5;
const DISC_HOVER: f32 = 0.7;

pub struct ArtworkActions {
    id: SharedString,
    group: SharedString,
    play: Option<OnPlay>,
    menu: Option<BuildMenu>,
    compact: bool,
    now_playing: Option<bool>,
}

/// Play and More over artwork, shown while the pointer is over the ancestor `.group(group)`.
/// Put it in a `relative()`, `overflow_hidden()` box with the artwork, rounded as the artwork
/// is; it fills the box.
#[track_caller]
pub fn artwork_actions(id: impl Into<SharedString>, group: impl Into<SharedString>) -> ArtworkActions {
    ArtworkActions { id: id.into(), group: group.into(), play: None, menu: None, compact: false, now_playing: None }
}

impl ArtworkActions {
    pub fn on_play(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.play = Some(Rc::new(f));
        self
    }

    pub fn menu(mut self, build: impl Fn(ContextMenu, &mut Window, &mut App) -> ContextMenu + 'static) -> Self {
        self.menu = Some(Rc::new(build));
        self
    }

    /// A row's artwork: the Play glyph alone in the middle, More left to the row. Without
    /// `on_play` the glyph only shows, and a click goes to the row (a list row plays on select).
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// Marks the artwork as the track that is playing (`Some(true)`) or paused on
    /// (`Some(false)`): it stays dimmed under the now-playing bars, which give way to the
    /// buttons under the pointer, and a row's glyph becomes Pause while it plays.
    pub fn now_playing(mut self, playing: Option<bool>) -> Self {
        self.now_playing = playing;
        self
    }
}

fn disc(id: SharedString, name: &'static str, glyph: Icon) -> Div {
    div()
        .id(id)
        .role(Role::Button, name)
        .size(px(DISC))
        .rounded_full()
        .bg(black().opacity(DISC_FILL))
        .hover(|s| s.bg(black().opacity(DISC_HOVER)))
        .flex()
        .items_center()
        .justify_center()
        .text_color(white())
        .child(icon(glyph, GLYPH).text_color(white()))
}

impl IntoElement for ArtworkActions {
    fn into_any_element(self) -> AnyElement {
        let ArtworkActions { id, group, play, menu, compact, now_playing } = self;
        let child = |suffix: &str| SharedString::from(format!("{id}.{suffix}"));
        // A marked artwork keeps its dim at rest; what the pointer brings fades in over it.
        let marked = now_playing.is_some();
        let reveal = |d: Div| if marked { d.reveal_on_hover(group.clone()) } else { d };
        let mark = now_playing.map(|playing| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .opacity(1.)
                .group_hover(group.clone(), |s| s.opacity(0.))
                .child(equalizer(if compact { ROW_BARS } else { DISC }, playing).text_color(white()))
        });
        let layer = div().id(id.clone()).absolute().top_0().left_0().size_full().bg(black().opacity(DIM));
        let layer = if marked { layer } else { layer.reveal_on_hover(group.clone()) };
        let play = play.map(|f| move |_: &TapEvent, w: &mut Window, cx: &mut App| f(w, cx));
        if compact {
            let (glyph, name) = match now_playing {
                Some(true) => (Icon::Pause, "Pause"),
                _ => (Icon::Play, "Play"),
            };
            let glyph = icon(glyph, ROW_GLYPH).text_color(white());
            let glyph = reveal(div().id(child("play")).role(Role::Button, name).child(glyph));
            return layer
                .flex()
                .items_center()
                .justify_center()
                .text_color(white())
                .children(mark)
                .child(match play {
                    Some(play) => glyph.on_tap(play),
                    None => glyph,
                })
                .into_any_element();
        }
        let more = menu.map(|build| {
            let key = child("more");
            disc(key.clone(), "More", Icon::Ellipsis).on_tap(move |_, window, cx| {
                let menu = build(ContextMenu::default(), window, cx);
                let Some((node, frame)) = cx.node_with_id(window.id(), &key) else { return };
                let below = frame.map_or(DISC, |f| f.size.height);
                crate::services::show_context_menu(cx, window.id(), node, 0., below, menu);
            })
        });
        layer
            .flex()
            .flex_col()
            .justify_end()
            .children(mark)
            .child(
                reveal(
                    h_flex()
                        .id(child("buttons"))
                        .w_full()
                        .p(px(INSET))
                        .justify_between()
                        .child(match play {
                            Some(play) => disc(child("play"), "Play", Icon::Play).on_tap(play),
                            None => div().id(child("play")),
                        })
                        .children(more),
                ),
            )
            .into_any_element()
    }
}

/// A More button revealed while the pointer is over the ancestor `.group(group)`: the end of a
/// song row in Music.
#[track_caller]
pub fn hover_menu_button(
    id: impl Into<SharedString>,
    group: impl Into<SharedString>,
    build: impl Fn(ContextMenu, &mut Window, &mut App) -> ContextMenu + 'static,
) -> Div {
    let id: SharedString = id.into();
    div()
        .flex_none()
        .reveal_on_hover(group)
        .child(crate::controls::menu_button(id, Icon::Ellipsis, build).tooltip("More"))
}
