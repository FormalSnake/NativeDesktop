//! Media transport: `seek_bar` (a scrubber with the buffered range, elapsed and remaining time)
//! and `player_bar` (transport buttons, now-playing art and text, seek bar and volume, in a
//! full or compact layout).
//!
//! Controlled: the app owns playback and passes a [`PlaybackClock`]; the bar extrapolates it
//! every frame on its own, so a playing track costs the app no renders between position
//! reports. While the user drags, the bar shows the drag and ignores the clock; on release it
//! fires one committed [`Seeked`] and keeps showing that position until the app's clock catches
//! up (or [`SEEK.settle`](SeekMetrics::settle) passes), so the thumb never snaps back while the
//! player seeks.
//!
//! There is no system scrubber with a buffered range on any platform, so the seek bar is drawn
//! from theme tokens everywhere; the buttons, labels and volume slider in `player_bar` are the
//! platform's own controls. Commands map one to one onto the OS media session's
//! (`nd-system`'s `MediaCommand`), so one handler serves the bar and the media keys.

use std::rc::Rc;

use nd_core::time::{Duration, Instant};
use nd_ui::prelude::*;
use nd_ui::semantics::{Response, SemanticAction, Semantics, WidgetSemantics};
use nd_ui::{Key, RichText, Role, StyleRefinement, span};

use super::clock::{PlaybackClock, format_clock, parse_clock};
use super::image::image;
use super::pipeline::ImageSource;
use crate::controls::{ButtonExt, icon_button};
use crate::forms::{LabelTone, SliderChanged, TextStyle, guard, label, leaf, not_taken, slider};
use crate::icon::Icon;

/// Seek bar geometry: the component's design, the same on every backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeekMetrics {
    pub track: f32,
    /// Track thickness while hovered or dragged.
    pub track_active: f32,
    pub thumb: f32,
    /// Height the bar takes, and answers the pointer in, without its time labels.
    pub hit: f32,
    /// Space between the track and the time labels.
    pub label_gap: f32,
    /// Space between the track and the time labels beside it.
    pub beside_gap: f32,
    /// How long a committed seek is shown while the app's clock has not caught up.
    pub settle: Duration,
    /// How close the clock must come to a committed seek to count as caught up.
    pub caught_up: Duration,
}

/// The pill layout's now-playing block, from Music.app on macOS 26 and 27.
pub struct PillMetrics {
    /// The capsule's height.
    pub height: f32,
    pub art: f32,
    pub art_radius: f32,
    /// Between the art and the two lines' frames; the glyphs' side bearing adds about 3 pt, giving
    /// Music's 8 pt from art to ink.
    pub art_gap: f32,
    /// Between the title and the artist line: Music's baselines sit 16 pt apart.
    pub line_gap: f32,
    /// Between the art and the seek bar's track.
    pub seek_gap: f32,
    /// The widest the block grows: about half the capsule in Music.
    pub song_width: f32,
    /// The point sizes of the play and skip glyphs; AppKit sizes a button's symbol from its font.
    pub play_glyph: f32,
    pub skip_glyph: f32,
}

pub const PILL: PillMetrics = PillMetrics {
    height: 52.,
    art: 34.,
    art_radius: 4.,
    art_gap: 5.,
    line_gap: 2.,
    seek_gap: 4.,
    song_width: 368.,
    play_glyph: 22.,
    skip_glyph: 17.,
};

pub const SEEK: SeekMetrics = SeekMetrics {
    track: 4.,
    track_active: 6.,
    thumb: 12.,
    hit: 16.,
    label_gap: 2.,
    beside_gap: 8.,
    settle: Duration::from_millis(1500),
    caught_up: Duration::from_millis(750),
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SeekPhase {
    /// The user is dragging; the player may preview but should not seek yet.
    Scrubbing,
    /// Released, or set by automation or the keyboard: seek now.
    #[default]
    Committed,
}

/// The user moved the playback position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seeked {
    pub position: Duration,
    pub phase: SeekPhase,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SeekBarProps {
    pub clock: PlaybackClock,
    /// `None` while the length is unknown (a live stream): the bar shows no thumb and takes no
    /// seeks.
    pub duration: Option<Duration>,
    /// How far the player has data, drawn under the played part.
    pub buffered: Option<Duration>,
    /// Elapsed and remaining time under the track.
    pub show_times: bool,
    /// The times sit on the track's row instead, elapsed before it and the length after it:
    /// a player bar's seek row, as GNOME Music and Music's mini player lay it out.
    pub times_beside: bool,
    /// What one arrow key moves.
    pub step: Duration,
    /// Accessible name.
    pub label: SharedString,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

impl SeekBarProps {
    pub fn seekable(&self) -> bool {
        self.enabled && self.duration.is_some_and(|d| !d.is_zero())
    }

    /// `position` as a fraction of the duration.
    pub fn fraction(&self, position: Duration) -> f32 {
        match self.duration {
            Some(d) if !d.is_zero() => (position.as_secs_f64() / d.as_secs_f64()).clamp(0., 1.) as f32,
            _ => 0.,
        }
    }

    pub fn position_of(&self, fraction: f32) -> Duration {
        self.duration.map_or(Duration::ZERO, |d| d.mul_f32(fraction.clamp(0., 1.)))
    }

    /// The width each time label beside the track takes, in points: room for `m:ss`, or
    /// `h:mm:ss` once the track runs an hour, in caption-sized tabular digits.
    pub fn beside_width(&self) -> f32 {
        if self.duration.is_some_and(|d| d.as_secs() >= 3600) { 52. } else { 40. }
    }

    /// How far in from each side the track starts when the times sit beside it.
    pub fn track_inset(&self) -> f32 {
        if self.times_beside { self.beside_width() + SEEK.beside_gap } else { 0. }
    }

    /// The buffered fraction, never behind `played`.
    pub fn buffered_fraction(&self, played: f32) -> f32 {
        self.buffered.map_or(played, |b| self.fraction(b).max(played))
    }
}

/// Widget kind of `seek_bar`.
pub struct SeekBarKind;

impl WidgetSemantics for SeekBarProps {
    fn semantics(&self) -> Semantics {
        let position = self.clock.now(self.duration);
        let value = match self.duration {
            Some(d) => format!("{} of {}", format_clock(position), format_clock(d)),
            None => format_clock(position),
        };
        Semantics::new(Role::Slider, self.label.to_string()).value(value).enabled(self.seekable())
    }

    fn respond(&self, action: &SemanticAction) -> Option<Response> {
        let SemanticAction::SetValue(text) = action else { return None };
        if let Some(refused) = guard(self.enabled) {
            return Some(refused);
        }
        let Some(duration) = self.duration else { return Some(not_taken("the length is unknown")) };
        Some(match parse_clock(text, Some(duration)) {
            Some(p) => Ok(Box::new(Seeked { position: p.min(duration), phase: SeekPhase::Committed })),
            None => not_taken(&format!("\"{text}\" is not a time")),
        })
    }
}

/// The seek bar's interaction state, the same on every backend: a drag in progress and a
/// committed seek waiting for the app's clock.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeekModel {
    drag: Option<f32>,
    pending: Option<(Duration, Instant)>,
    hovered: bool,
}

impl SeekModel {
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn hovered(&self) -> bool {
        self.hovered
    }

    /// Whether the track draws thick and the thumb shows.
    pub fn active(&self) -> bool {
        self.hovered || self.drag.is_some()
    }

    pub fn set_hovered(&mut self, hovered: bool) -> bool {
        std::mem::replace(&mut self.hovered, hovered) != hovered
    }

    /// The position to draw at `now`.
    pub fn shown(&mut self, props: &SeekBarProps, now: Instant) -> Duration {
        if let Some(f) = self.drag {
            return props.position_of(f);
        }
        let clock = props.clock.position_at(now, props.duration);
        if let Some((target, at)) = self.pending {
            let gap = clock.abs_diff(target);
            if gap > SEEK.caught_up && now.saturating_duration_since(at) < SEEK.settle {
                return target;
            }
            self.pending = None;
        }
        clock
    }

    /// A press at `fraction` along the track.
    pub fn press(&mut self, props: &SeekBarProps, fraction: f32) -> Option<Seeked> {
        if !props.seekable() {
            return None;
        }
        self.drag = Some(fraction.clamp(0., 1.));
        Some(Seeked { position: props.position_of(fraction), phase: SeekPhase::Scrubbing })
    }

    pub fn moved(&mut self, props: &SeekBarProps, fraction: f32) -> Option<Seeked> {
        let drag = self.drag.as_mut()?;
        let fraction = fraction.clamp(0., 1.);
        if (*drag - fraction).abs() < f32::EPSILON {
            return None;
        }
        *drag = fraction;
        Some(Seeked { position: props.position_of(fraction), phase: SeekPhase::Scrubbing })
    }

    /// The release: the one committed seek of a drag.
    pub fn release(&mut self, props: &SeekBarProps, now: Instant) -> Option<Seeked> {
        let fraction = self.drag.take()?;
        let position = props.position_of(fraction);
        self.pending = Some((position, now));
        Some(Seeked { position, phase: SeekPhase::Committed })
    }

    /// An arrow key: `direction` steps of `props.step` from what is shown.
    pub fn step(&mut self, props: &SeekBarProps, direction: i32, now: Instant) -> Option<Seeked> {
        if !props.seekable() || self.drag.is_some() {
            return None;
        }
        let duration = props.duration?;
        let from = self.shown(props, now);
        let position = if direction < 0 {
            from.saturating_sub(props.step * direction.unsigned_abs())
        } else {
            (from + props.step * direction as u32).min(duration)
        };
        self.pending = Some((position, now));
        Some(Seeked { position, phase: SeekPhase::Committed })
    }

    /// Elapsed and remaining labels for the shown position; elapsed and the length when the
    /// times sit beside the track.
    pub fn times(&mut self, props: &SeekBarProps, now: Instant) -> (String, String) {
        let shown = self.shown(props, now);
        let end = match props.duration {
            Some(d) if props.times_beside => format_clock(d),
            Some(d) => format!("-{}", format_clock(d.saturating_sub(shown))),
            None if props.times_beside => "--:--".to_owned(),
            None => String::new(),
        };
        (format_clock(shown), end)
    }
}

pub struct SeekBar(NativeElement<SeekBarProps>);

/// A playback scrubber over `clock`. Give it the track's `duration`; it fires [`Seeked`].
pub fn seek_bar(id: impl Into<SharedString>, clock: PlaybackClock) -> SeekBar {
    let props = SeekBarProps {
        clock,
        duration: None,
        buffered: None,
        show_times: false,
        times_beside: false,
        step: Duration::from_secs(5),
        label: "Playback position".into(),
        enabled: true,
        tooltip: None,
        style: StyleRefinement::default(),
    };
    SeekBar(NativeElement::described::<SeekBarKind>(Key::Id(id.into()), props).w_full())
}

impl SeekBar {
    pub fn duration(mut self, duration: Option<Duration>) -> Self {
        self.0.props_mut().duration = duration;
        self
    }

    pub fn buffered(mut self, buffered: Option<Duration>) -> Self {
        self.0.props_mut().buffered = buffered;
        self
    }

    pub fn show_times(mut self) -> Self {
        self.0.props_mut().show_times = true;
        self
    }

    /// Elapsed time before the track and the length after it, on one row.
    pub fn times_beside(mut self) -> Self {
        let props = self.0.props_mut();
        props.show_times = true;
        props.times_beside = true;
        self
    }

    pub fn step(mut self, step: Duration) -> Self {
        self.0.props_mut().step = step;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.0.props_mut().label = label.into();
        self
    }

    pub fn on_seek(self, f: impl Fn(&Seeked, &mut Window, &mut App) + 'static) -> Self {
        SeekBar(self.0.on(f))
    }
}

leaf!(SeekBar, SeekBarProps);

/// What a `player_bar` shows.
#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub title: SharedString,
    pub subtitle: Option<SharedString>,
    pub artwork: Option<ImageSource>,
    pub clock: PlaybackClock,
    pub duration: Option<Duration>,
    pub buffered: Option<Duration>,
    /// 0 to 1.
    pub volume: f32,
    pub muted: bool,
    pub can_previous: bool,
    pub can_next: bool,
}

impl NowPlaying {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            artwork: None,
            clock: PlaybackClock::default(),
            duration: None,
            buffered: None,
            volume: 1.,
            muted: false,
            can_previous: true,
            can_next: true,
        }
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub fn artwork(mut self, artwork: impl Into<ImageSource>) -> Self {
        self.artwork = Some(artwork.into());
        self
    }

    pub fn clock(mut self, clock: PlaybackClock) -> Self {
        self.clock = clock;
        self
    }

    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    pub fn buffered(mut self, buffered: Duration) -> Self {
        self.buffered = Some(buffered);
        self
    }

    pub fn volume(mut self, volume: f32, muted: bool) -> Self {
        self.volume = volume.clamp(0., 1.);
        self.muted = muted;
        self
    }

    pub fn playing(&self) -> bool {
        self.clock.playing
    }
}

/// What the user asked the player to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransportCommand {
    TogglePlay,
    Previous,
    Next,
    /// A drag in progress; preview only.
    Scrub(Duration),
    Seek(Duration),
    SetVolume(f32),
    ToggleMute,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TransportLayout {
    /// A window's bottom bar: the seek bar with its times across the top, then transport at
    /// the leading edge, what is playing in the middle and volume at the trailing edge, as
    /// GNOME Music and YouTube Music lay it out.
    #[default]
    Full,
    /// Art, title and play and next with a thin seek bar under: a sidebar or mini player.
    Compact,
    /// One row for a capsule, as macOS Music floats it over its content: transport at the leading
    /// end, the song on two lines over a thin seek bar in the middle, the item's controls and
    /// volume at the trailing end. Sits in a [`crate::bottom_accessory`].
    Pill,
}

impl TransportLayout {
    /// The bar the platform's music players use: the pill on macOS, the bottom bar elsewhere.
    pub fn platform() -> Self {
        if cfg!(target_os = "macos") { Self::Pill } else { Self::Full }
    }
}

type OnCommand = Rc<dyn Fn(&TransportCommand, &mut Window, &mut App)>;
type OnOpen = Rc<dyn Fn(&mut Window, &mut App)>;

/// A now-playing bar built from the platform's controls and a [`seek_bar`].
pub struct PlayerBar {
    id: SharedString,
    now: NowPlaying,
    layout: TransportLayout,
    on_command: Option<OnCommand>,
    on_open: Option<OnOpen>,
    accessories: Vec<AnyElement>,
    trailing: Vec<AnyElement>,
    before_transport: Vec<AnyElement>,
    after_transport: Vec<AnyElement>,
    style: StyleRefinement,
}

pub fn player_bar(id: impl Into<SharedString>, now: NowPlaying) -> PlayerBar {
    PlayerBar {
        id: id.into(),
        now,
        layout: TransportLayout::Full,
        on_command: None,
        on_open: None,
        accessories: Vec::new(),
        trailing: Vec::new(),
        before_transport: Vec::new(),
        after_transport: Vec::new(),
        style: StyleRefinement::default(),
    }
}

impl PlayerBar {
    pub fn layout(mut self, layout: TransportLayout) -> Self {
        self.layout = layout;
        self
    }

    pub fn compact(self) -> Self {
        self.layout(TransportLayout::Compact)
    }

    pub fn on_command(mut self, f: impl Fn(&TransportCommand, &mut Window, &mut App) + 'static) -> Self {
        self.on_command = Some(Rc::new(f));
        self
    }

    /// A tap on the artwork and titles opens the app's now-playing view (the expanded player).
    pub fn on_open(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_open = Some(Rc::new(f));
        self
    }

    /// A control after the titles that acts on the playing item: rating, its menu. Full layout
    /// only.
    pub fn accessory(mut self, element: impl IntoElement) -> Self {
        self.accessories.push(element.into_any_element());
        self
    }

    /// A control after the volume: repeat, shuffle, the queue. Full and pill layouts.
    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing.push(element.into_any_element());
        self
    }

    /// A control before previous, where Music keeps shuffle. Full and pill layouts.
    pub fn before_transport(mut self, element: impl IntoElement) -> Self {
        self.before_transport.push(element.into_any_element());
        self
    }

    /// A control after next, where Music keeps repeat. Full and pill layouts.
    pub fn after_transport(mut self, element: impl IntoElement) -> Self {
        self.after_transport.push(element.into_any_element());
        self
    }
}

impl Styled for PlayerBar {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// The volume icon for a level, as the system's own volume menu picks it.
pub fn volume_icon(volume: f32, muted: bool) -> Icon {
    if muted || volume <= 0. {
        Icon::VolumeX
    } else if volume < 0.34 {
        Icon::Volume
    } else if volume < 0.67 {
        Icon::Volume1
    } else {
        Icon::Volume2
    }
}

impl IntoElement for PlayerBar {
    fn into_any_element(self) -> AnyElement {
        let PlayerBar {
            id,
            now,
            layout,
            on_command,
            on_open,
            accessories,
            trailing,
            before_transport,
            after_transport,
            style,
        } = self;
        let send = move |command: TransportCommand| {
            let on = on_command.clone();
            move |window: &mut Window, cx: &mut App| {
                if let Some(on) = &on {
                    on(&command, window, cx);
                }
            }
        };
        let child = |suffix: &str| SharedString::from(format!("{id}.{suffix}"));
        let playing = now.playing();
        let play = icon_button(child("play"), if playing { Icon::Pause } else { Icon::Play })
            .tooltip(if playing { "Pause" } else { "Play" })
            .on_click({
                let f = send(TransportCommand::TogglePlay);
                move |_, w, cx| f(w, cx)
            });
        let next = icon_button(child("next"), Icon::SkipForward).tooltip("Next").on_click({
            let f = send(TransportCommand::Next);
            move |_, w, cx| f(w, cx)
        });
        let seek = {
            let on = on_command_of(&send);
            seek_bar(child("seek"), now.clock).duration(now.duration).buffered(now.buffered).on_seek(
                move |e: &Seeked, w, cx| {
                    let command = match e.phase {
                        SeekPhase::Scrubbing => TransportCommand::Scrub(e.position),
                        SeekPhase::Committed => TransportCommand::Seek(e.position),
                    };
                    on(command)(w, cx);
                },
            )
        };
        let titles = v_flex()
            .flex_1()
            .min_w_0()
            .child(label(now.title.clone()).text_style(TextStyle::Headline))
            .when_some(now.subtitle.clone(), |el, s| {
                el.child(label(s).text_style(TextStyle::Subheadline).tone(LabelTone::Secondary))
            });
        let art = |size: f32| {
            let el = match &now.artwork {
                Some(source) => image(source.clone()).id(child("art")),
                None => super::image::image_placeholder().id(child("art")),
            };
            el.alt(now.title.clone()).size(px(size)).flex_none().rounded_md()
        };
        let mut root = match layout {
            TransportLayout::Compact => v_flex()
                .id(id.clone())
                .w_full()
                .gap_1()
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(art(36.))
                        .child(titles)
                        .child(play)
                        .child(next.disabled(!now.can_next)),
                )
                .child(seek),
            TransportLayout::Pill => {
                let previous = icon_button(child("previous"), Icon::SkipBack)
                    .tooltip("Previous")
                    .disabled(!now.can_previous)
                    .on_click({
                        let f = send(TransportCommand::Previous);
                        move |_, w, cx| f(w, cx)
                    });
                let mute = icon_button(child("mute"), volume_icon(now.volume, now.muted))
                    .tooltip(if now.muted { "Unmute" } else { "Mute" })
                    .on_click({
                        let f = send(TransportCommand::ToggleMute);
                        move |_, w, cx| f(w, cx)
                    });
                let volume = {
                    let on = on_command_of(&send);
                    slider(child("volume"), if now.muted { 0. } else { now.volume })
                        .label("Volume")
                        .w(px(72.))
                        .on_change(move |e: &SliderChanged, w, cx| on(TransportCommand::SetVolume(e.value))(w, cx))
                };
                let idle = now.title.is_empty() && now.artwork.is_none();
                // Music's mini player, measured against Music.app: 34 pt art, the two lines 8 pt
                // after it and centred against it, the title semibold, the progress line 4 pt
                // under both from the art's leading edge.
                let song =
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .justify_center()
                        .gap(px(PILL.seek_gap))
                        .child(
                            h_flex()
                                .id(child("now"))
                                .w_full()
                                .min_w_0()
                                .gap(px(PILL.art_gap))
                                .items_center()
                                .child(art(PILL.art).rounded(px(PILL.art_radius)))
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap(px(PILL.line_gap))
                                        .child(
                                            label(now.title.clone())
                                                .rich(RichText::new([span(now.title.clone()).semibold()]))
                                                .text_style(TextStyle::Subheadline)
                                                .truncate(),
                                        )
                                        .when_some(now.subtitle.clone(), |el, s| {
                                            el.child(
                                                label(s)
                                                    .text_style(TextStyle::Subheadline)
                                                    .tone(LabelTone::Secondary)
                                                    .truncate(),
                                            )
                                        }),
                                )
                                .when_some(on_open, |row, open| {
                                    row.role(Role::Button, format!("{}, open player", now.title))
                                        .on_tap(move |_, w, cx| open(w, cx))
                                }),
                        )
                        // The seek bar's hit area is taller than its track; the block counts the track
                        // only, so art, titles and track centre as one unit in the capsule.
                        .child(div().relative().w_full().h(px(SEEK.track)).child(
                            div().absolute().left_0().right_0().top(px((SEEK.track - SEEK.hit) / 2.)).child(seek),
                        ));
                // Music draws play larger than skip and skip larger than its other controls.
                let play = play.text_size(px(PILL.play_glyph));
                let previous = previous.text_size(px(PILL.skip_glyph));
                let next = next.text_size(px(PILL.skip_glyph));
                // Transport and the trailing controls keep their size; the song truncates first.
                h_flex()
                    .id(id.clone())
                    .w_full()
                    .gap_3()
                    .items_center()
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_1()
                            .items_center()
                            .children(before_transport)
                            .child(previous)
                            .child(play)
                            .child(next.disabled(!now.can_next))
                            .children(after_transport),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .justify_center()
                            .when(!idle, |row| row.child(song.max_w(px(PILL.song_width)))),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_1()
                            .items_center()
                            .children(accessories)
                            .children(trailing)
                            .child(mute)
                            .child(volume),
                    )
            }
            TransportLayout::Full => {
                let previous = icon_button(child("previous"), Icon::SkipBack)
                    .tooltip("Previous")
                    .disabled(!now.can_previous)
                    .on_click({
                        let f = send(TransportCommand::Previous);
                        move |_, w, cx| f(w, cx)
                    });
                let mute = icon_button(child("mute"), volume_icon(now.volume, now.muted))
                    .tooltip(if now.muted { "Unmute" } else { "Mute" })
                    .on_click({
                        let f = send(TransportCommand::ToggleMute);
                        move |_, w, cx| f(w, cx)
                    });
                let volume = {
                    let on = on_command_of(&send);
                    slider(child("volume"), if now.muted { 0. } else { now.volume })
                        .label("Volume")
                        .w(px(96.))
                        .on_change(move |e: &SliderChanged, w, cx| on(TransportCommand::SetVolume(e.value))(w, cx))
                };
                let idle = now.title.is_empty() && now.artwork.is_none();
                // The song fills the width between transport and volume, so its titles truncate
                // only once that runs out.
                let playing_now = h_flex()
                    .id(child("now"))
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .when(!idle, |row| row.child(art(44.)).child(titles))
                    .when_some(on_open.filter(|_| !idle), |row, open| {
                        row.role(Role::Button, format!("{}, open player", now.title))
                            .on_tap(move |_, w, cx| open(w, cx))
                    });
                let center = h_flex()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .gap_2()
                    .child(playing_now)
                    .when(!idle && !accessories.is_empty(), |row| {
                        row.child(h_flex().flex_none().gap_1().children(accessories))
                    });
                // The sides take only their controls' width.
                let side = || h_flex().flex_none().gap_1().items_center();
                v_flex().id(id.clone()).w_full().gap_1().child(seek.times_beside()).child(
                    h_flex()
                        .w_full()
                        .gap_3()
                        .items_center()
                        .child(
                            side()
                                .children(before_transport)
                                .child(previous)
                                .child(play)
                                .child(next.disabled(!now.can_next))
                                .children(after_transport),
                        )
                        .child(center)
                        .child(side().justify_end().child(mute).child(volume).children(trailing)),
                )
            }
        };
        nd_gpui::gpui::Refineable::refine(root.style(), &style);
        root.into_any_element()
    }
}

/// A command sender usable from any number of handlers.
fn on_command_of<F, G>(send: &F) -> impl Fn(TransportCommand) -> G + 'static
where
    F: Fn(TransportCommand) -> G + Clone + 'static,
{
    let send = send.clone();
    move |c| send(c)
}
