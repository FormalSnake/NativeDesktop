//! `lyrics_view`: time-synced lyrics that follow playback.
//!
//! The model here is backend-neutral and holds every rule; backends draw it. Rules, from
//! youtubemusic's `lyrics.rs` (its docs/lyrics.md):
//! - Rows: every line with text, plus an interlude row (a music note filling with progress) for
//!   an intro over [`INTRO_GAP`] and for a gap between lines of at least [`LONG_GAP`], or over
//!   [`INTRO_GAP`] when the lyrics carry end stamps.
//! - Lit set at time `t` (the clock plus [`LEAD`]): a line is active from its start to its end,
//!   or to the next main line's start when it has no end, and stays lit past its end while the
//!   next line starts within [`CARRY_GAP`]. The anchor is the last active main line, else the
//!   previous anchor, so the view never jumps back to the top in a gap.
//! - Words: line-synced lyrics get word starts spread by character count over the line, so
//!   every synced line fills word by word. A word's fill runs over its span, capped at
//!   [`MAX_WORD_SPAN`] for real word stamps.
//! - Follow: the anchor row sits at [`ANCHOR_AT`] of the viewport, tweened over [`SCROLL`]. A
//!   user scroll stops following and shows the resync button; the button, new lyrics, or
//!   `resync_after` of no scrolling resume it.
//! - Depth: rows fade and blur with their distance from the anchor through [`DEPTH_OPACITY`]
//!   and [`DEPTH_BLUR`], easing on the theme's slow timing as the anchor moves; lit rows draw
//!   in the primary ink at full size, the rest in the secondary ink at [`UNLIT_SCALE`]. While
//!   the user scrolls, every row is sharp and whole; resync brings the depth back.
//! - Unsynced lyrics are a plain column; a click on a synced row seeks to its start.

use std::sync::Arc;

use nd_core::time::{Duration, Instant};
use nd_ui::prelude::*;
use nd_ui::semantics::{Semantics, WidgetSemantics};
use nd_ui::{Key, Role, StyleRefinement};

use super::clock::PlaybackClock;
use crate::forms::leaf;

/// Rows are judged this far ahead of the clock, so a line lights as it is heard.
pub const LEAD: Duration = Duration::from_millis(100);
pub const CARRY_GAP: f64 = 3.0;
pub const INTRO_GAP: f64 = 3.0;
pub const LONG_GAP: f64 = 5.0;
/// A real word stamp fills over at most this long.
pub const MAX_WORD_SPAN: f64 = 1.2;
/// A word with nothing after it ends this long after it starts.
pub const LAST_WORD: f64 = 0.35;
/// A line with no end and nothing after it is taken to last this long.
pub const LINE_GUESS: f64 = 7.0;
/// The anchor row's top, as a fraction of the viewport height.
pub const ANCHOR_AT: f32 = 0.42;
/// The follow scroll: 500 ms on a curve that overshoots slightly.
pub const SCROLL: Duration = Duration::from_millis(500);
pub const SCROLL_CURVE: [f32; 4] = [0.38, 1.21, 0.22, 1.0];
pub const UNLIT_SCALE: f32 = 0.85;
/// Opacity by distance from the anchor in viewport spans: on the anchor, then a third, two
/// thirds and a full span away.
pub const DEPTH_OPACITY: [f32; 4] = [1.0, 0.7, 0.45, 0.25];
/// Blur radius in points on the same steps: the lit line sharp, the rest out of focus.
pub const DEPTH_BLUR: [f32; 4] = [0.0, 1.5, 3.0, 4.5];

#[derive(Clone, Debug, PartialEq)]
pub struct LyricWord {
    pub start: Duration,
    pub text: SharedString,
    /// No space before the next word (a syllable of the same word).
    pub joins_next: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LyricLine {
    pub start: Duration,
    pub end: Option<Duration>,
    pub text: SharedString,
    /// Word stamps, when the source has them.
    pub words: Vec<LyricWord>,
    /// Backing vocals: judged on their own timing, drawn smaller.
    pub background: bool,
    /// The other voice of a duet, drawn on the trailing side.
    pub opposite: bool,
}

impl LyricLine {
    pub fn new(start: Duration, text: impl Into<SharedString>) -> Self {
        Self { start, end: None, text: text.into(), words: Vec::new(), background: false, opposite: false }
    }

    pub fn end(mut self, end: Duration) -> Self {
        self.end = Some(end);
        self
    }

    pub fn words(mut self, words: impl IntoIterator<Item = (Duration, &'static str)>) -> Self {
        self.words =
            words.into_iter().map(|(start, text)| LyricWord { start, text: text.into(), joins_next: false }).collect();
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
    /// `false` for plain text with no timing.
    pub synced: bool,
    /// Where they came from, shown under unsynced lyrics.
    pub source: Option<SharedString>,
}

impl Lyrics {
    pub fn synced(lines: impl IntoIterator<Item = LyricLine>) -> Self {
        Self { lines: lines.into_iter().collect(), synced: true, source: None }
    }

    pub fn plain(text: &str) -> Self {
        let lines = text.lines().map(|l| LyricLine::new(Duration::ZERO, l.to_owned())).collect();
        Self { lines, synced: false, source: None }
    }
}

/// One word as rows draw it: its text, where its fill starts and how long it takes.
#[derive(Clone, Debug, PartialEq)]
pub struct RowWord {
    pub text: SharedString,
    pub start: f64,
    pub span: f64,
    /// Followed by a space.
    pub space_after: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Line,
    Background,
    /// A gap with no words: a note that fills with the gap's progress.
    Interlude,
}

/// One row of the view.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub kind: RowKind,
    /// Index into [`Lyrics::lines`]; `None` for an interlude.
    pub line: Option<usize>,
    pub text: SharedString,
    pub start: f64,
    pub end: Option<f64>,
    pub words: Vec<RowWord>,
    pub opposite: bool,
}

impl Row {
    pub fn is_main(&self) -> bool {
        self.kind != RowKind::Background
    }
}

fn secs(d: Duration) -> f64 {
    d.as_secs_f64()
}

/// The rows for `lyrics`: lines with text in time order, interludes in the long gaps, and word
/// fills for every synced line.
pub fn rows(lyrics: &Lyrics) -> Vec<Row> {
    let mut lines: Vec<(usize, &LyricLine)> =
        lyrics.lines.iter().enumerate().filter(|(_, l)| !l.text.trim().is_empty() || !l.words.is_empty()).collect();
    if !lyrics.synced {
        return lines
            .into_iter()
            .map(|(i, l)| Row {
                kind: RowKind::Line,
                line: Some(i),
                text: l.text.clone(),
                start: 0.,
                end: None,
                words: Vec::new(),
                opposite: false,
            })
            .collect();
    }
    lines.sort_by_key(|a| a.1.start);
    let has_ends = lines.iter().any(|(_, l)| l.end.is_some());
    let main_starts: Vec<f64> = lines.iter().filter(|(_, l)| !l.background).map(|(_, l)| secs(l.start)).collect();
    let next_main = |after: f64| main_starts.iter().copied().find(|s| *s > after);
    let mut out = Vec::new();
    let mut last_end: Option<f64> = None;
    for (n, (i, line)) in lines.iter().enumerate() {
        let start = secs(line.start);
        let end = line.end.map(secs).filter(|e| *e > start);
        if !line.background {
            let gap_from = match last_end {
                None => 0.,
                Some(e) => e.min(start),
            };
            let gap = start - gap_from;
            let wanted = match last_end {
                None => gap > INTRO_GAP,
                Some(_) => gap >= LONG_GAP || (has_ends && gap > INTRO_GAP),
            };
            if wanted {
                out.push(Row {
                    kind: RowKind::Interlude,
                    line: None,
                    text: SharedString::default(),
                    start: gap_from,
                    end: Some(start),
                    words: Vec::new(),
                    opposite: false,
                });
            }
        }
        let after = lines.get(n + 1).map(|(_, l)| secs(l.start));
        let line_end = end.or_else(|| next_main(start)).unwrap_or(start + LINE_GUESS);
        let words = if line.words.is_empty() {
            synthesize(&line.text, start, line_end)
        } else {
            stamped(&line.words, line_end)
        };
        out.push(Row {
            kind: if line.background { RowKind::Background } else { RowKind::Line },
            line: Some(*i),
            text: line.text.clone(),
            start,
            end,
            words,
            opposite: line.opposite,
        });
        if !line.background {
            let estimate =
                end.or_else(|| line.words.last().map(|w| secs(w.start) + LAST_WORD)).unwrap_or(start + LINE_GUESS);
            let estimate = after.map_or(estimate, |a| estimate.min(a.max(start)));
            last_end = Some(last_end.map_or(estimate, |l: f64| l.max(estimate)));
        }
    }
    out
}

fn stamped(words: &[LyricWord], line_end: f64) -> Vec<RowWord> {
    words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let start = secs(w.start);
            let next = words.get(i + 1).map_or(line_end.max(start + LAST_WORD), |n| secs(n.start));
            RowWord {
                text: w.text.clone(),
                start,
                span: (next - start).clamp(0., MAX_WORD_SPAN),
                space_after: !w.joins_next && i + 1 < words.len(),
            }
        })
        .collect()
}

/// Word starts for a line with only a line stamp: spread by character count from `start` to
/// `end`, so the fill runs at the pace the line is sung.
fn synthesize(text: &str, start: f64, end: f64) -> Vec<RowWord> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.is_empty() || text.contains('\n') {
        return Vec::new();
    }
    let total: usize = parts.iter().map(|p| p.chars().count()).sum::<usize>().max(1);
    let span = (end - start).max(0.);
    let mut before = 0usize;
    let mut out = Vec::with_capacity(parts.len());
    for (i, part) in parts.iter().enumerate() {
        let len = part.chars().count();
        let s = start + span * before as f64 / total as f64;
        let e = start + span * (before + len) as f64 / total as f64;
        out.push(RowWord {
            text: SharedString::from(part.to_string()),
            start: s,
            span: e - s,
            space_after: i + 1 < parts.len(),
        });
        before += len;
    }
    out
}

/// How far a word has filled at `t`, 0 to 1.
pub fn word_fill(word: &RowWord, t: f64) -> f32 {
    if t < word.start {
        return 0.;
    }
    if word.span <= 0. {
        return 1.;
    }
    ((t - word.start) / word.span).clamp(0., 1.) as f32
}

/// An interlude's progress at `t`, 0 to 1.
pub fn interlude_fill(row: &Row, t: f64) -> f32 {
    let end = row.end.unwrap_or(row.start);
    if end <= row.start {
        return 1.;
    }
    ((t - row.start) / (end - row.start)).clamp(0., 1.) as f32
}

/// Which rows are lit at a moment, and which one the view centres.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lit {
    pub anchor: usize,
    /// Every lit row, ascending.
    pub rows: Vec<usize>,
}

impl Lit {
    pub fn contains(&self, row: usize) -> bool {
        self.rows.binary_search(&row).is_ok()
    }
}

/// The lit rows at `t`. `previous` is the anchor last time, kept through gaps.
pub fn lit_at(rows: &[Row], t: f64, previous: usize) -> Lit {
    let mains: Vec<usize> = (0..rows.len()).filter(|i| rows[*i].is_main()).collect();
    let next_main_start = |i: usize| mains.iter().find(|m| **m > i).map(|m| rows[*m].start);
    let active = |i: usize| -> bool {
        let row = &rows[i];
        if t < row.start {
            return false;
        }
        let next = next_main_start(i);
        match row.end {
            Some(end) if t < end => true,
            Some(end) => next.is_some_and(|n| n - end <= CARRY_GAP && t < n),
            None => next.is_none_or(|n| t < n),
        }
    };
    let mut lit = Vec::new();
    let mut anchor = None;
    for &m in &mains {
        if active(m) {
            lit.push(m);
            anchor = Some(m);
        }
    }
    let main_lit = anchor.is_some();
    for (i, row) in rows.iter().enumerate() {
        if row.kind == RowKind::Background && main_lit && active(i) {
            lit.push(i);
        }
    }
    // Duet rows light only while a main line does.
    if !main_lit {
        lit.retain(|i| !rows[*i].opposite);
    }
    lit.sort_unstable();
    let anchor = anchor.or_else(|| lit.last().copied()).unwrap_or(previous.min(rows.len().saturating_sub(1)));
    Lit { anchor, rows: lit }
}

/// The opacity of a row `distance` rows from the anchor, with `span` rows filling the side of
/// the viewport it is on.
pub fn depth_opacity(distance: usize, span: f32) -> f32 {
    depth_step(&DEPTH_OPACITY, distance, span)
}

/// The blur radius, in points, of a row `distance` rows from the anchor: the depth of field
/// around the line being sung.
pub fn depth_blur(distance: usize, span: f32) -> f32 {
    depth_step(&DEPTH_BLUR, distance, span)
}

fn depth_step(steps: &[f32; 4], distance: usize, span: f32) -> f32 {
    let at = (distance as f32 * 3. / span.max(1.)).min(3.);
    let i = at.floor() as usize;
    if i >= 3 {
        return steps[3];
    }
    let f = at - i as f32;
    steps[i] + (steps[i + 1] - steps[i]) * f
}

/// A synced row's opacity and blur. While the user scrolls every row is sharp and whole, so the
/// lyrics read as a page; following brings the depth of field back.
pub fn row_depth(lit: bool, distance: usize, span: f32, following: bool) -> (f32, f32) {
    if lit || !following {
        return (1., 0.);
    }
    (depth_opacity(distance, span), depth_blur(distance, span))
}

/// Where the anchor row's top sits in the viewport, for a viewport `height` points tall.
pub fn anchor_y(height: f32) -> f32 {
    (height * ANCHOR_AT).round()
}

/// The vertical offset that puts row top `row_top` at the anchor line.
pub fn follow_offset(row_top: f32, height: f32) -> f32 {
    row_top - anchor_y(height)
}

/// Follow mode: whether the view tracks playback, and when the user last scrolled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Follow {
    following: bool,
    scrolled_at: Option<Instant>,
}

impl Default for Follow {
    fn default() -> Self {
        Self { following: true, scrolled_at: None }
    }
}

impl Follow {
    pub fn following(&self) -> bool {
        self.following
    }

    /// The user scrolled the view. Returns `true` when that ended following.
    pub fn user_scrolled(&mut self, now: Instant) -> bool {
        self.scrolled_at = Some(now);
        std::mem::replace(&mut self.following, false)
    }

    /// The resync button, or new lyrics. Returns `true` when following resumed.
    pub fn resync(&mut self) -> bool {
        self.scrolled_at = None;
        !std::mem::replace(&mut self.following, true)
    }

    /// Resumes following once `after` passed since the last scroll. Returns `true` when it did.
    pub fn tick(&mut self, now: Instant, after: Option<Duration>) -> bool {
        match (self.following, self.scrolled_at, after) {
            (false, Some(at), Some(after)) if now.saturating_duration_since(at) >= after => self.resync(),
            _ => false,
        }
    }
}

/// The lit state and follow mode a view keeps between frames, the same on every backend.
#[derive(Clone, Debug, Default)]
pub struct LyricsModel {
    pub rows: Vec<Row>,
    pub lit: Lit,
    pub follow: Follow,
    lyrics: Option<Arc<Lyrics>>,
}

impl LyricsModel {
    /// New props: rebuilds rows when the lyrics changed, and then follows again from the top.
    /// Returns `true` when the lyrics changed.
    pub fn set_lyrics(&mut self, lyrics: &Arc<Lyrics>) -> bool {
        if self.lyrics.as_ref().is_some_and(|l| Arc::ptr_eq(l, lyrics) || **l == **lyrics) {
            return false;
        }
        self.rows = rows(lyrics);
        self.lit = Lit::default();
        self.follow.resync();
        self.lyrics = Some(lyrics.clone());
        true
    }

    pub fn synced(&self) -> bool {
        self.lyrics.as_ref().is_some_and(|l| l.synced)
    }

    /// The time rows are judged at.
    pub fn time(props: &LyricsProps, now: Instant) -> f64 {
        (props.clock.position_at(now, props.duration) + LEAD).as_secs_f64()
    }

    /// Recomputes the lit set at `now`. Returns `true` when it changed.
    pub fn advance(&mut self, props: &LyricsProps, now: Instant) -> bool {
        if !self.synced() || self.rows.is_empty() {
            return false;
        }
        let lit = lit_at(&self.rows, Self::time(props, now), self.lit.anchor);
        let changed = lit != self.lit;
        self.lit = lit;
        changed
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LyricsProps {
    pub lyrics: Arc<Lyrics>,
    pub clock: PlaybackClock,
    pub duration: Option<Duration>,
    /// Follow resumes this long after the user's last scroll; `None` waits for the button.
    pub resync_after: Option<Duration>,
    /// Shown when there are no lines ("No lyrics for this song.").
    pub empty_text: SharedString,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

/// The user clicked a synced row: seek to `start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LyricClicked {
    /// Index into [`Lyrics::lines`]; `None` for an interlude.
    pub line: Option<usize>,
    pub start: Duration,
}

/// Follow mode changed: the user scrolled away, or it resumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LyricsFollowChanged {
    pub following: bool,
}

/// Widget kind of `lyrics_view`.
pub struct LyricsKind;

impl WidgetSemantics for LyricsProps {
    fn semantics(&self) -> Semantics {
        let rows = rows(&self.lyrics);
        let value = if self.lyrics.synced && !rows.is_empty() {
            let lit = lit_at(&rows, LyricsModel::time(self, Instant::now()), 0);
            rows.get(lit.anchor).map(|r| r.text.to_string()).unwrap_or_default()
        } else {
            String::new()
        };
        Semantics::new(Role::List, "Lyrics").value(value).enabled(self.enabled)
    }
}

pub struct LyricsView(NativeElement<LyricsProps>);

/// Lyrics following `clock`. Size it with the style; it fills what it is given.
pub fn lyrics_view(id: impl Into<SharedString>, lyrics: impl Into<Arc<Lyrics>>, clock: PlaybackClock) -> LyricsView {
    let props = LyricsProps {
        lyrics: lyrics.into(),
        clock,
        duration: None,
        resync_after: None,
        empty_text: "No lyrics for this song.".into(),
        enabled: true,
        tooltip: None,
        style: StyleRefinement::default(),
    };
    LyricsView(NativeElement::described::<LyricsKind>(Key::Id(id.into()), props))
}

impl LyricsView {
    pub fn duration(mut self, duration: Option<Duration>) -> Self {
        self.0.props_mut().duration = duration;
        self
    }

    pub fn resync_after(mut self, after: Duration) -> Self {
        self.0.props_mut().resync_after = Some(after);
        self
    }

    pub fn empty_text(mut self, text: impl Into<SharedString>) -> Self {
        self.0.props_mut().empty_text = text.into();
        self
    }

    pub fn on_line_click(self, f: impl Fn(&LyricClicked, &mut Window, &mut App) + 'static) -> Self {
        LyricsView(self.0.on(f))
    }

    pub fn on_follow_change(self, f: impl Fn(&LyricsFollowChanged, &mut Window, &mut App) + 'static) -> Self {
        LyricsView(self.0.on(f))
    }
}

leaf!(LyricsView, LyricsProps);

/// The cubic bezier `curve` at `x` (0 to 1), as CSS evaluates `cubic-bezier`.
pub fn bezier(curve: [f32; 4], x: f32) -> f32 {
    let [x1, y1, x2, y2] = curve;
    let x = x.clamp(0., 1.);
    let sample = |a: f32, b: f32, t: f32| {
        let u = 1. - t;
        3. * u * u * t * a + 3. * u * t * t * b + t * t * t
    };
    let mut lo = 0f32;
    let mut hi = 1f32;
    let mut t = x;
    for _ in 0..24 {
        let v = sample(x1, x2, t);
        if (v - x).abs() < 1e-4 {
            break;
        }
        if v < x {
            lo = t;
        } else {
            hi = t;
        }
        t = (lo + hi) / 2.;
    }
    sample(y1, y2, t)
}
