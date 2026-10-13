//! The backend-neutral virtualization model (ARCHITECTURE.md 7): rows by id, the height tree,
//! the scroll position as an anchor, follow mode and jump targets. Presenters report measured
//! heights and user scrolls into it and ask it where every row goes; nothing here renders.
//!
//! Position is never raw pixels. It is an anchor, the row at the viewport's top edge and how far
//! into that row the edge sits, so a height change or an insert above the anchor moves nothing on
//! screen, and a prepend keeps the view still. Pixel offsets are derived per layout from the
//! anchor and the height tree, and clamped only against rows that are measured, so an estimate
//! that is too small can never pull the end of the list out of reach.

use std::collections::HashMap;
use std::ops::Range;

use crate::heights::Heights;

/// A row's identity inside one list. Stable for as long as its key stays in the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RowId(pub u64);

/// Where rows sit when they do not fill the viewport.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Alignment {
    #[default]
    Top,
    /// Chat: rows rest against the bottom edge and the list opens at its end.
    Bottom,
}

/// What the list does when rows arrive at its end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Follow {
    #[default]
    None,
    /// Stays pinned to the end while the user is there; scrolling away stops it, scrolling back
    /// to the end starts it again.
    Tail,
}

/// Where `scroll_to` puts a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
    /// Only scrolls when the row is not fully visible, to whichever edge is closer.
    Nearest,
}

/// Fixed-size cells laid out in lanes: a grid instead of a list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSpec {
    pub item_width: f64,
    pub item_height: f64,
    pub gap: f64,
}

/// Rows removed (old indices) and inserted (new indices) by one `set_rows`, ascending, for
/// presenters that animate native inserts and removals. A row that moved is in both.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub removed: Vec<usize>,
    pub inserted: Vec<usize>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.inserted.is_empty()
    }
}

/// A tree row's place in the hierarchy, for presenters that draw disclosure natively.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowMeta {
    pub depth: u16,
    pub expandable: bool,
    pub expanded: bool,
}

/// One placed row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub index: usize,
    pub row: RowId,
    /// Lane (grid column); 0 in a list.
    pub lane: usize,
    /// Top edge relative to the viewport's top.
    pub y: f64,
    pub height: f64,
    /// The height is a measurement at the current width, not an estimate.
    pub measured: bool,
}

/// What a presenter needs to paint one frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    /// Content offset at the viewport's top edge; negative when bottom-aligned rows do not fill
    /// the viewport.
    pub scroll_top: f64,
    pub total: f64,
    /// Rows intersecting the viewport, in order.
    pub visible: Vec<Placed>,
    /// Rows the owner should have rendered: the viewport plus overscan on both sides.
    pub render: Range<usize>,
    /// Some visible row is still an estimate: the presenter has no output for it yet.
    pub unmeasured: bool,
    pub at_start: bool,
    pub at_end: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Anchor {
    row: RowId,
    offset: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Target {
    row: RowId,
    align: Align,
    offset: f64,
    /// Layouts this target has been pending; it settles after a few even if rows never measure.
    tries: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Anim {
    from: f64,
    start: Option<f64>,
}

/// A user scroll within this distance of the end counts as at the end.
const END_EPSILON: f64 = 1.0;
/// Jump animations cover at most this many viewports; farther jumps cut straight there.
const ANIMATE_VIEWPORTS: f64 = 3.0;
const ANIMATION_SECONDS: f64 = 0.25;
/// A pending target is settled after this many layouts even if its rows never measured.
const TARGET_TRIES: u8 = 8;

#[derive(Clone, Debug)]
pub struct Core {
    ids: Vec<RowId>,
    index: HashMap<RowId, usize>,
    /// One entry per line: per row in a list, per lane row in a grid.
    heights: Heights,
    /// Per line: measured at the current width.
    fresh: Vec<bool>,
    estimate: f64,
    overscan: f64,
    alignment: Alignment,
    follow: Follow,
    tail: bool,
    grid: Option<GridSpec>,
    lanes: usize,
    width: Option<f64>,
    viewport: f64,
    anchor: Option<Anchor>,
    target: Option<Target>,
    anim: Option<Anim>,
    now: Option<f64>,
    diffs: Vec<Diff>,
    generation: u64,
    meta: Vec<RowMeta>,
}

impl Default for Core {
    fn default() -> Self {
        Self::new()
    }
}

impl Core {
    pub fn new() -> Self {
        Self {
            ids: Vec::new(),
            index: HashMap::new(),
            heights: Heights::default(),
            fresh: Vec::new(),
            estimate: 32.,
            overscan: 1.0,
            alignment: Alignment::Top,
            follow: Follow::None,
            tail: false,
            grid: None,
            lanes: 1,
            width: None,
            viewport: 0.,
            anchor: None,
            target: None,
            anim: None,
            now: None,
            diffs: Vec::new(),
            generation: 0,
            meta: Vec::new(),
        }
    }

    // Options.

    /// Height assumed for a row until it is measured.
    pub fn set_estimate(&mut self, estimate: f64) {
        let estimate = estimate.max(1.);
        if estimate == self.estimate {
            return;
        }
        self.estimate = estimate;
        if self.grid.is_none() {
            for line in 0..self.fresh.len() {
                if !self.fresh[line] {
                    self.heights.set(line, estimate);
                }
            }
        }
    }

    pub fn estimate(&self) -> f64 {
        self.estimate
    }

    /// Rows rendered beyond each edge of the viewport, in viewports.
    pub fn set_overscan(&mut self, viewports: f64) {
        self.overscan = viewports.max(0.);
    }

    pub fn set_alignment(&mut self, alignment: Alignment) {
        self.alignment = alignment;
    }

    pub fn alignment(&self) -> Alignment {
        self.alignment
    }

    /// `Follow::Tail` engages at once when the list is at its end (or bottom-aligned and not
    /// yet scrolled).
    pub fn set_follow(&mut self, follow: Follow) {
        self.follow = follow;
        self.tail = follow == Follow::Tail && (self.anchor.is_none() || self.at_end());
        if follow == Follow::None {
            self.tail = false;
        }
    }

    pub fn follow(&self) -> Follow {
        self.follow
    }

    /// True while `Follow::Tail` keeps the list pinned to its end.
    pub fn following(&self) -> bool {
        self.follow == Follow::Tail && self.tail
    }

    /// Lays rows out as fixed-size cells in as many lanes as the width holds.
    pub fn set_grid(&mut self, grid: Option<GridSpec>) {
        if self.grid == grid {
            return;
        }
        self.grid = grid;
        self.reflow();
    }

    pub fn grid(&self) -> Option<GridSpec> {
        self.grid
    }

    pub fn lanes(&self) -> usize {
        self.lanes
    }

    /// Bumped by every `set_rows`; presenters compare it to know the rows changed.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    // Rows.

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn ids(&self) -> &[RowId] {
        &self.ids
    }

    pub fn row(&self, index: usize) -> Option<RowId> {
        self.ids.get(index).copied()
    }

    pub fn index_of(&self, row: RowId) -> Option<usize> {
        self.index.get(&row).copied()
    }

    fn line_of(&self, index: usize) -> usize {
        index / self.lanes
    }

    fn line_count(&self) -> usize {
        self.ids.len().div_ceil(self.lanes)
    }

    /// The row's height: measured, or the estimate.
    pub fn height(&self, index: usize) -> f64 {
        self.heights.get(self.line_of(index))
    }

    pub fn is_measured(&self, index: usize) -> bool {
        self.fresh[self.line_of(index)]
    }

    /// Content offset of the row's top edge.
    pub fn top(&self, index: usize) -> f64 {
        self.heights.top(self.line_of(index))
    }

    pub fn total(&self) -> f64 {
        self.heights.total()
    }

    pub fn viewport(&self) -> f64 {
        self.viewport
    }

    pub fn width(&self) -> Option<f64> {
        self.width
    }

    /// Replaces the rows, keyed: rows that stay keep their height and the anchor stays on its
    /// row, so nothing on screen moves unless a visible row came or went. Returns the native
    /// diff, also queued for [`Self::take_diffs`].
    pub fn set_rows(&mut self, rows: Vec<RowId>) -> Diff {
        let old_ids = std::mem::take(&mut self.ids);
        let old_index = std::mem::take(&mut self.index);
        let old_heights = std::mem::take(&mut self.heights);
        let old_fresh = std::mem::take(&mut self.fresh);
        let old_lanes = self.lanes;
        let old_top = |ix: usize| old_heights.top(ix / old_lanes);

        self.index = rows.iter().enumerate().map(|(i, r)| (*r, i)).collect();
        debug_assert_eq!(self.index.len(), rows.len(), "row ids are unique");
        self.ids = rows;

        // The anchor row left: the nearest survivor takes over at the same screen position.
        if let Some(anchor) = self.anchor
            && !self.index.contains_key(&anchor.row)
        {
            let at = old_index[&anchor.row];
            let a_top = old_top(at);
            let after = old_ids[at + 1..].iter().position(|r| self.index.contains_key(r)).map(|p| at + 1 + p);
            let before = old_ids[..at].iter().rposition(|r| self.index.contains_key(r));
            self.anchor = match (after, before) {
                (Some(s), _) => Some(Anchor { row: old_ids[s], offset: anchor.offset - (old_top(s) - a_top) }),
                (None, Some(p)) => Some(Anchor { row: old_ids[p], offset: anchor.offset + (a_top - old_top(p)) }),
                (None, None) => None,
            };
        }
        if let Some(t) = self.target
            && !self.index.contains_key(&t.row)
        {
            self.target = None;
            self.anim = None;
        }

        if self.grid.is_some() {
            self.reflow();
        } else {
            let mut heights = Vec::with_capacity(self.ids.len());
            let mut fresh = Vec::with_capacity(self.ids.len());
            for id in &self.ids {
                match old_index.get(id) {
                    Some(&old) => {
                        heights.push(old_heights.get(old));
                        fresh.push(old_fresh[old]);
                    }
                    None => {
                        heights.push(self.estimate);
                        fresh.push(false);
                    }
                }
            }
            self.heights = Heights::from_vec(heights);
            self.fresh = fresh;
        }
        self.normalize_anchor(false);

        let diff = diff(&old_ids, &old_index, &self.ids, &self.index);
        self.generation += 1;
        if !diff.is_empty() {
            self.diffs.push(diff.clone());
        }
        diff
    }

    /// Tree rows' hierarchy, by index, after `set_rows`. Empty for lists.
    pub fn set_meta(&mut self, meta: Vec<RowMeta>) {
        debug_assert!(meta.is_empty() || meta.len() == self.ids.len());
        self.meta = meta;
    }

    pub fn meta(&self, index: usize) -> RowMeta {
        self.meta.get(index).copied().unwrap_or_default()
    }

    /// Diffs since the last call, oldest first.
    pub fn take_diffs(&mut self) -> Vec<Diff> {
        std::mem::take(&mut self.diffs)
    }

    /// Records the row's height as measured at the current width. Returns whether it changed.
    /// Grid cells have the grid's size and ignore it.
    pub fn report_height(&mut self, row: RowId, height: f64) -> bool {
        if self.grid.is_some() || !height.is_finite() {
            return false;
        }
        let Some(&ix) = self.index.get(&row) else { return false };
        let height = height.max(0.);
        let was_fresh = std::mem::replace(&mut self.fresh[ix], true);
        if (self.heights.get(ix) - height).abs() < 0.01 {
            return !was_fresh;
        }
        self.heights.set(ix, height);
        true
    }

    /// The row's content may have changed height: the next layout trusts it only once the
    /// presenter measures it again.
    pub fn invalidate(&mut self, row: RowId) {
        if let Some(&ix) = self.index.get(&row)
            && self.grid.is_none()
        {
            self.fresh[ix] = false;
        }
    }

    /// The viewport the presenter shows rows in. A new width makes every measurement stale; the
    /// old heights stay as estimates until rows are measured again.
    pub fn set_viewport(&mut self, width: f64, height: f64) {
        self.viewport = height.max(0.);
        if self.width != Some(width) {
            self.width = Some(width);
            if self.grid.is_some() {
                self.reflow();
            } else {
                self.fresh.iter_mut().for_each(|f| *f = false);
            }
        }
    }

    fn reflow(&mut self) {
        match self.grid {
            Some(g) => {
                let width = self.width.unwrap_or(g.item_width);
                self.lanes = (((width + g.gap) / (g.item_width + g.gap)).floor() as usize).max(1);
                let lines = self.line_count();
                self.heights = Heights::from_vec(vec![g.item_height + g.gap; lines]);
                self.fresh = vec![true; lines];
            }
            None => {
                if self.lanes != 1 {
                    self.lanes = 1;
                    self.heights = Heights::from_vec(vec![self.estimate; self.ids.len()]);
                    self.fresh = vec![false; self.ids.len()];
                }
            }
        }
    }

    // Position.

    /// The range the scroll offset may take: `(lo, hi)`.
    fn bounds(&self) -> (f64, f64) {
        let hi = self.total() - self.viewport;
        if hi >= 0. {
            (0., hi)
        } else if self.alignment == Alignment::Bottom {
            (hi, hi)
        } else {
            (0., 0.)
        }
    }

    fn clamp(&self, y: f64) -> f64 {
        let (lo, hi) = self.bounds();
        y.clamp(lo, hi)
    }

    fn anchor_top(&self) -> Option<f64> {
        let a = self.anchor?;
        Some(self.top(*self.index.get(&a.row)?) + a.offset)
    }

    fn target_top(&self, t: &Target) -> Option<f64> {
        let ix = *self.index.get(&t.row)?;
        let (top, h, vp) = (self.top(ix), self.height(ix), self.viewport);
        Some(match t.align {
            Align::Start | Align::Nearest => top + t.offset,
            Align::Center => top + h / 2. - vp / 2. + t.offset,
            Align::End => top + h - vp + t.offset,
        })
    }

    /// Where the list rests, ignoring a running animation.
    fn resting_top(&self) -> f64 {
        if self.following() {
            return self.bounds().1;
        }
        if let Some(y) = self.target.as_ref().and_then(|t| self.target_top(t)) {
            return self.clamp(y);
        }
        match self.anchor_top() {
            Some(y) => self.clamp(y),
            None if self.alignment == Alignment::Bottom => self.bounds().1,
            None => self.bounds().0,
        }
    }

    fn eased(&self, to: f64) -> f64 {
        let (Some(anim), Some(now)) = (self.anim, self.now) else { return to };
        let Some(start) = anim.start else { return anim.from };
        let t = ((now - start) / ANIMATION_SECONDS).clamp(0., 1.);
        let e = 1. - (1. - t).powi(3);
        anim.from + (to - anim.from) * e
    }

    /// The content offset at the viewport's top edge now.
    pub fn scroll_top(&self) -> f64 {
        self.eased(self.resting_top())
    }

    pub fn at_end(&self) -> bool {
        self.scroll_top() >= self.bounds().1 - END_EPSILON
    }

    pub fn at_start(&self) -> bool {
        self.scroll_top() <= self.bounds().0 + END_EPSILON
    }

    /// Re-expresses the position as the row at the viewport's top edge. `clamp` also pulls it
    /// into the scrollable range, which is only done once the rows that bound it are measured.
    fn normalize_anchor(&mut self, clamp: bool) {
        let Some(y) = self.anchor_top() else { return };
        let y = if clamp { self.clamp(y) } else { y };
        self.set_anchor_at(y);
    }

    fn set_anchor_at(&mut self, y: f64) {
        let Some(line) = self.heights.index_at(y) else {
            self.anchor = None;
            return;
        };
        let ix = (line * self.lanes).min(self.ids.len() - 1);
        let offset = y - self.top(ix);
        self.anchor = Some(Anchor { row: self.ids[ix], offset });
    }

    /// A user scroll by `dy` points (positive scrolls towards the end). Stops a pending jump,
    /// and drives tail follow: away from the end stops it, reaching the end starts it.
    pub fn scroll_by(&mut self, dy: f64) -> bool {
        let from = self.scroll_top();
        self.scroll_to_offset(from + dy)
    }

    /// A user scroll to content offset `y`, as a native scroll view reports it.
    pub fn scroll_to_offset(&mut self, y: f64) -> bool {
        let from = self.scroll_top();
        let to = self.clamp(y);
        self.target = None;
        self.anim = None;
        if self.is_empty() {
            return false;
        }
        let hi = self.bounds().1;
        if self.follow == Follow::Tail {
            if to < from && to < hi - END_EPSILON {
                self.tail = false;
            } else if to >= hi - END_EPSILON {
                self.tail = true;
            }
        }
        self.set_anchor_at(to);
        (to - from).abs() > f64::EPSILON
    }

    /// Scrolls `row` into place: `align` picks the edge, `offset` moves the result (negative
    /// leaves room above the row). Animated on the frame clock for short distances.
    pub fn scroll_to(&mut self, row: RowId, align: Align, offset: f64, animate: bool) {
        let Some(&ix) = self.index.get(&row) else { return };
        let current = self.scroll_top();
        let align = match align {
            Align::Nearest => {
                let (top, h) = (self.top(ix), self.height(ix));
                if top >= current && top + h <= current + self.viewport {
                    return;
                } else if top < current {
                    Align::Start
                } else {
                    Align::End
                }
            }
            a => a,
        };
        self.tail = false;
        let target = Target { row, align, offset, tries: 0 };
        let to = self.target_top(&target).map_or(current, |y| self.clamp(y));
        self.anim = (animate && (to - current).abs() <= ANIMATE_VIEWPORTS * self.viewport.max(1.))
            .then_some(Anim { from: current, start: None });
        self.target = Some(target);
    }

    pub fn scroll_to_start(&mut self) {
        self.target = None;
        self.anim = None;
        self.tail = false;
        self.anchor = self.ids.first().map(|&row| Anchor { row, offset: 0. });
    }

    /// Jumps to the end; with `Follow::Tail` the list stays there as rows arrive.
    pub fn scroll_to_end(&mut self) {
        self.anim = None;
        if self.follow == Follow::Tail {
            self.tail = true;
        }
        self.target = self.ids.last().map(|&row| Target { row, align: Align::End, offset: 0., tries: 0 });
    }

    /// The frame clock, in seconds. Jump animations advance with it.
    pub fn set_time(&mut self, now: f64) {
        self.now = Some(now);
    }

    /// True while a jump animation runs: the presenter asks for another frame.
    pub fn animating(&self) -> bool {
        self.anim.is_some()
    }

    /// Lays the viewport out from the current heights. Call after reporting the heights of
    /// every rendered row. Settles a pending jump once the rows it depends on are measured, and
    /// re-anchors on the row now at the top edge.
    pub fn layout(&mut self) -> Layout {
        if let (Some(anim), Some(now)) = (self.anim.as_mut(), self.now)
            && anim.start.is_none()
        {
            anim.start = Some(now);
        }
        let scroll_top = self.scroll_top();
        let visible = self.place(scroll_top, self.viewport);
        let unmeasured = visible.iter().any(|p| !p.measured);
        let render = self.render_range_at(scroll_top);
        let (lo, hi) = self.bounds();
        let layout = Layout {
            scroll_top,
            total: self.total(),
            visible,
            render,
            unmeasured,
            at_start: scroll_top <= lo + END_EPSILON,
            at_end: scroll_top >= hi - END_EPSILON,
        };

        let done = match (self.anim, self.now) {
            (Some(Anim { start: Some(start), .. }), Some(now)) => now - start >= ANIMATION_SECONDS,
            (Some(_), _) => self.now.is_none(),
            (None, _) => true,
        };
        if done {
            self.anim = None;
        }
        if let Some(t) = self.target.as_mut() {
            t.tries = t.tries.saturating_add(1);
            if self.anim.is_none() && (!unmeasured || t.tries >= TARGET_TRIES) {
                let y = self.resting_top();
                self.target = None;
                self.set_anchor_at(y);
            }
        } else if self.anim.is_none() && !self.following() && !unmeasured {
            self.normalize_anchor(true);
        }
        layout
    }

    /// The rows in the viewport now, without settling anything: what a presenter must lay out
    /// before [`Self::layout`].
    pub fn visible(&self) -> Vec<Placed> {
        self.place(self.scroll_top(), self.viewport)
    }

    /// The row's height is a measurement at the current width.
    pub fn is_fresh(&self, row: RowId) -> bool {
        self.index.get(&row).is_some_and(|&i| self.is_measured(i))
    }

    fn place(&self, scroll_top: f64, viewport: f64) -> Vec<Placed> {
        let mut out = Vec::new();
        if self.ids.is_empty() || viewport <= 0. {
            return out;
        }
        let Some(first_line) = self.heights.index_at(scroll_top.max(0.)) else { return out };
        let end = scroll_top + viewport;
        for line in first_line..self.line_count() {
            let top = self.heights.top(line);
            if top >= end {
                break;
            }
            let height = self.heights.get(line);
            for lane in 0..self.lanes {
                let index = line * self.lanes + lane;
                let Some(&row) = self.ids.get(index) else { break };
                out.push(Placed { index, row, lane, y: top - scroll_top, height, measured: self.fresh[line] });
            }
        }
        out
    }

    fn render_range_at(&self, scroll_top: f64) -> Range<usize> {
        if self.ids.is_empty() {
            return 0..0;
        }
        let pad = self.viewport.max(self.estimate) * self.overscan;
        let first = self.heights.index_at((scroll_top - pad).max(0.)).unwrap_or(0);
        let last = self.heights.index_at(scroll_top + self.viewport + pad).unwrap_or(0);
        let start = first * self.lanes;
        let end = ((last + 1) * self.lanes).min(self.ids.len());
        start..end
    }

    /// Rows the owner should have rendered now: the viewport plus overscan, spanning a running
    /// jump animation from where it is to where it lands.
    pub fn render_range(&self) -> Range<usize> {
        let resting = self.render_range_at(self.resting_top());
        if self.anim.is_none() {
            return resting;
        }
        let here = self.render_range_at(self.scroll_top());
        here.start.min(resting.start)..here.end.max(resting.end)
    }
}

/// The native diff between two row orders: removed rows by old index, inserted by new index.
pub fn diff_rows(old: &[RowId], new: &[RowId]) -> Diff {
    let old_index: HashMap<RowId, usize> = old.iter().enumerate().map(|(i, r)| (*r, i)).collect();
    let new_index: HashMap<RowId, usize> = new.iter().enumerate().map(|(i, r)| (*r, i)).collect();
    diff(old, &old_index, new, &new_index)
}

/// Removed rows by old index and inserted rows by new index, ascending. Survivors that changed
/// relative order are moves, reported as removed and inserted: the longest run of survivors in
/// old order stays put.
fn diff(old: &[RowId], old_index: &HashMap<RowId, usize>, new: &[RowId], new_index: &HashMap<RowId, usize>) -> Diff {
    let survivors: Vec<usize> = new.iter().filter_map(|r| old_index.get(r).copied()).collect();
    let keep = longest_increasing(&survivors);
    let mut removed: Vec<usize> =
        old.iter().enumerate().filter(|(_, r)| !new_index.contains_key(r)).map(|(i, _)| i).collect();
    let mut inserted: Vec<usize> = Vec::new();
    for (i, r) in new.iter().enumerate() {
        match old_index.get(r) {
            None => inserted.push(i),
            Some(o) if !keep.contains(o) => {
                inserted.push(i);
                removed.push(*o);
            }
            Some(_) => {}
        }
    }
    removed.sort_unstable();
    Diff { removed, inserted }
}

/// The values of one longest strictly increasing subsequence.
fn longest_increasing(xs: &[usize]) -> std::collections::HashSet<usize> {
    let mut tails: Vec<usize> = Vec::new();
    let mut tail_ix: Vec<usize> = Vec::new();
    let mut prev = vec![usize::MAX; xs.len()];
    for (i, &x) in xs.iter().enumerate() {
        let pos = tails.partition_point(|&t| t < x);
        if pos > 0 {
            prev[i] = tail_ix[pos - 1];
        }
        if pos == tails.len() {
            tails.push(x);
            tail_ix.push(i);
        } else {
            tails[pos] = x;
            tail_ix[pos] = i;
        }
    }
    let mut out = std::collections::HashSet::new();
    let mut at = tail_ix.last().copied();
    while let Some(i) = at {
        out.insert(xs[i]);
        at = (prev[i] != usize::MAX).then_some(prev[i]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(r: Range<u64>) -> Vec<RowId> {
        r.map(RowId).collect()
    }

    #[test]
    fn diff_reports_moves_as_remove_and_insert() {
        let mut c = Core::new();
        c.set_rows(ids(0..5));
        let d = c.set_rows(vec![RowId(0), RowId(3), RowId(1), RowId(2), RowId(5)]);
        assert_eq!(d.removed, vec![3, 4]);
        assert_eq!(d.inserted, vec![1, 4]);
    }
}
