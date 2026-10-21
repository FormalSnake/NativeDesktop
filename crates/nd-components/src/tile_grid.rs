//! `tile_grid()`: tiles in rows the user reorders by dragging, as Arc's pinned favourites. The
//! grid takes as many columns as fit tiles of at least the minimum width, up to a maximum, and
//! sizes tiles to share the width; a short last row keeps the pitch. A dragged tile follows the
//! pointer, takes the slot under it while the others slide aside, and settles where it is
//! dropped or slides home on Escape or a release elsewhere. Built from elements, so it is the
//! same on every backend; the tiles' content is the app's.
//!
//! Tiles drag as [`DragItem`]s of the grid's kind, so another window's grid or list takes them
//! too; drags of the grid's [`accepts`](TileGrid::accepts_drops) kinds open a gap where they
//! would land. The order is the app's: a tile dropped in its own grid reports [`TileMoved`], a
//! foreign drop reports [`Dropped`] with the slot.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use nd_ui::prelude::*;
use nd_ui::{DragEnded, DragItem, DragOver, DragOverPhase, DropSlot, Dropped, Point, Resized, WindowId};

/// One tile: its id and what it shows.
pub struct Tile {
    pub id: SharedString,
    pub content: AnyElement,
}

pub fn tile(id: impl Into<SharedString>, content: impl IntoElement) -> Tile {
    Tile { id: id.into(), content: content.into_any_element() }
}

/// A tile of this grid was dropped at `index`, counted in the order without it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileMoved {
    pub id: SharedString,
    pub index: usize,
}

/// How the grid lays tiles out for a width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileMetrics {
    pub columns: usize,
    pub tile: (f32, f32),
    /// From one tile's origin to the next one's.
    pub pitch: (f32, f32),
}

impl TileMetrics {
    pub fn new(width: f32, max_columns: usize, min_width: f32, aspect: f32, gap: f32) -> Self {
        let fit = ((width + gap) / (min_width + gap)).floor().max(1.) as usize;
        let columns = fit.min(max_columns.max(1));
        let w = ((width - gap * (columns as f32 - 1.)) / columns as f32).max(0.);
        let h = (w * aspect).round();
        Self { columns, tile: (w, h), pitch: (w + gap, h + gap) }
    }

    /// The origin of slot `i`.
    pub fn origin(&self, i: usize) -> (f32, f32) {
        ((i % self.columns) as f32 * self.pitch.0, (i / self.columns) as f32 * self.pitch.1)
    }

    /// The grid's height for `count` slots.
    pub fn height(&self, count: usize) -> f32 {
        let rows = count.div_ceil(self.columns);
        if rows == 0 { 0. } else { rows as f32 * self.pitch.1 - (self.pitch.1 - self.tile.1) }
    }

    /// The slot under `local`, of `count` slots.
    pub fn slot_at(&self, local: Point, count: usize) -> usize {
        let col = ((local.x / self.pitch.0).floor().max(0.) as usize).min(self.columns - 1);
        let row = (local.y / self.pitch.1).floor().max(0.) as usize;
        (row * self.columns + col).min(count.saturating_sub(1))
    }
}

#[derive(Clone, Debug, Default)]
struct GridDrag {
    id: SharedString,
    /// The pointer's travel since the press.
    translation: (f32, f32),
    /// The slot under the pointer while it is over the grid.
    slot: Option<usize>,
}

#[derive(Clone, Debug, Default)]
struct GridState {
    width: Option<f32>,
    /// The width the tiles were last placed for.
    placed: Option<f32>,
    drag: Option<GridDrag>,
    /// Where a drag from elsewhere would land.
    gap: Option<usize>,
}

#[derive(Default)]
struct TileGrids(HashMap<(WindowId, SharedString), GridState>);

type Handler<E> = Rc<dyn Fn(&E, &mut Window, &mut App)>;

pub struct TileGrid {
    id: SharedString,
    tiles: Vec<Tile>,
    max_columns: usize,
    min_width: f32,
    aspect: f32,
    gap: f32,
    kind: Option<SharedString>,
    accepts: Vec<SharedString>,
    on_move: Option<Handler<TileMoved>>,
    on_drop: Option<Handler<Dropped>>,
    on_drag_end: Option<Handler<DragEnded>>,
}

/// A grid of at most four columns of tiles at least 40 points wide, 0.74 as tall as wide.
pub fn tile_grid(id: impl Into<SharedString>) -> TileGrid {
    TileGrid {
        id: id.into(),
        tiles: Vec::new(),
        max_columns: 4,
        min_width: 40.,
        aspect: 0.74,
        gap: 8.,
        kind: None,
        accepts: Vec::new(),
        on_move: None,
        on_drop: None,
        on_drag_end: None,
    }
}

/// How long the others take to slide aside, and a tile to settle.
pub const TILE_SLIDE: Duration = Duration::from_millis(200);

impl TileGrid {
    pub fn tiles(mut self, tiles: impl IntoIterator<Item = Tile>) -> Self {
        self.tiles = tiles.into_iter().collect();
        self
    }

    pub fn max_columns(mut self, columns: usize) -> Self {
        self.max_columns = columns.max(1);
        self
    }

    pub fn min_tile_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }

    /// A tile's height over its width.
    pub fn aspect(mut self, aspect: f32) -> Self {
        self.aspect = aspect;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// Tiles drag as items of `kind`, reordering within the grid and onto any target taking it.
    pub fn draggable_tiles(mut self, kind: impl Into<SharedString>) -> Self {
        self.kind = Some(kind.into());
        self
    }

    /// Kinds from elsewhere the grid takes (a list's rows, to pin them).
    pub fn accepts_drops(mut self, kinds: impl IntoIterator<Item = impl Into<SharedString>>) -> Self {
        self.accepts = kinds.into_iter().map(Into::into).collect();
        self
    }

    pub fn on_move(mut self, f: impl Fn(&TileMoved, &mut Window, &mut App) + 'static) -> Self {
        self.on_move = Some(Rc::new(f));
        self
    }

    /// A drag from outside the grid was dropped on it; `slot.index` is where.
    pub fn on_drop(mut self, f: impl Fn(&Dropped, &mut Window, &mut App) + 'static) -> Self {
        self.on_drop = Some(Rc::new(f));
        self
    }

    /// A tile's drag is over (dropped elsewhere, cancelled, released outside the app).
    pub fn on_drag_end(mut self, f: impl Fn(&DragEnded, &mut Window, &mut App) + 'static) -> Self {
        self.on_drag_end = Some(Rc::new(f));
        self
    }

    fn build(self, window: &mut Window, app: &mut App) -> AnyElement {
        if !app.has_global::<TileGrids>() {
            app.set_global(TileGrids::default());
        }
        let key = (window.id(), self.id.clone());
        let view = app.rendering_view();
        let state = app.global::<TileGrids>().0.get(&key).cloned().unwrap_or_default();
        let width = state.width.unwrap_or(0.);
        // A new width reflows at once: tiles slide only when the order changes.
        let reflow = state.placed != state.width;
        if reflow {
            app.global_mut::<TileGrids>().0.entry(key.clone()).or_default().placed = state.width;
        }
        let m = TileMetrics::new(width, self.max_columns, self.min_width, self.aspect, self.gap);
        let ids: Vec<SharedString> = self.tiles.iter().map(|t| t.id.clone()).collect();

        // Each tile's slot as shown now.
        let dragged = state.drag.as_ref().filter(|d| ids.contains(&d.id));
        let mut order: Vec<Option<SharedString>> = ids.iter().cloned().map(Some).collect();
        if let Some(d) = dragged
            && let Some(slot) = d.slot
        {
            order.retain(|i| i.as_ref() != Some(&d.id));
            order.insert(slot.min(order.len()), Some(d.id.clone()));
        } else if let Some(gap) = state.gap {
            order.insert(gap.min(order.len()), None);
        }
        let slot_of: HashMap<SharedString, usize> =
            order.iter().enumerate().filter_map(|(i, id)| Some((id.clone()?, i))).collect();

        let notify = move |app: &mut App| {
            if let Some(view) = view {
                app.notify(view);
            }
        };
        let update = move |window: &Window, app: &mut App, f: &dyn Fn(&mut GridState)| {
            let key = (window.id(), key.1.clone());
            f(app.global_mut::<TileGrids>().0.entry(key).or_default());
            notify(app);
        };

        let count = order.len();
        let kind = self.kind.clone();
        let mut accepts = self.accepts.clone();
        accepts.extend(kind.clone());
        let mut children = Vec::with_capacity(self.tiles.len());
        for tile in self.tiles {
            let slot = slot_of.get(&tile.id).copied().unwrap_or(0);
            let (x, y) = m.origin(slot);
            let follow = dragged.filter(|d| d.id == tile.id);
            let (x, y) = match follow {
                // Follows the pointer from where it was picked up.
                Some(d) => {
                    let home = m.origin(ids.iter().position(|i| *i == d.id).unwrap_or(0));
                    (home.0 + d.translation.0, home.1 + d.translation.1)
                }
                None => (x, y),
            };
            let mut el = div()
                .id(tile.id.clone())
                .absolute()
                .top_0()
                .left_0()
                .w(px(m.tile.0))
                .h(px(m.tile.1))
                .offset(px(x), px(y));
            // A dragged tile tracks the pointer at once and slides when let go.
            if follow.is_none() && !reflow {
                el = el.animate(Animation::ease_out(TILE_SLIDE));
            }
            let mut el = el.child(tile.content);
            if let Some(kind) = &kind {
                let id = tile.id.clone();
                let (begin, end) = (update.clone(), update.clone());
                let on_end = self.on_drag_end.clone();
                el = el
                    .draggable(DragItem::new(kind.clone(), tile.id.clone()))
                    .on_drag(move |e: &DragEvent, window, app| {
                        let id = id.clone();
                        let translation = (e.translation.x, e.translation.y);
                        begin(window, app, &|s| match &mut s.drag {
                            Some(d) if d.id == id => d.translation = translation,
                            _ => s.drag = Some(GridDrag { id: id.clone(), translation, slot: None }),
                        });
                    })
                    .on_drag_end(move |e: &DragEnded, window, app| {
                        end(window, app, &|s| s.drag = None);
                        if let Some(f) = &on_end {
                            f(e, window, app);
                        }
                    });
            }
            children.push(el.into_any_element());
        }

        let (over, dropped) = (update.clone(), update.clone());
        let own: Rc<Vec<SharedString>> = Rc::new(ids);
        let (own_over, own_drop) = (own.clone(), own);
        let (own_kind, drop_kind) = (kind.clone(), kind.clone());
        let (on_move, on_drop) = (self.on_move.clone(), self.on_drop.clone());
        let tiles_now = count - usize::from(state.gap.is_some());
        div()
            .id(self.id)
            .relative()
            .w_full()
            .h(px(m.height(count)))
            .on_resize(move |e: &Resized, window, app| {
                let width = e.width;
                update(window, app, &|s| s.width = Some(width));
            })
            .accepts_drops(accepts)
            .on_drag_over(move |e: &DragOver, window, app| {
                let mine = own_kind.as_ref() == Some(&e.item.kind) && own_over.contains(&e.item.id);
                let inside = e.phase != DragOverPhase::Exited;
                let local = e.local;
                let id = e.item.id.clone();
                over(window, app, &|s| {
                    if mine {
                        if let Some(d) = s.drag.as_mut().filter(|d| d.id == id) {
                            d.slot = inside.then(|| m.slot_at(local, tiles_now));
                        }
                    } else {
                        s.gap = inside.then(|| m.slot_at(local, tiles_now + 1));
                    }
                });
            })
            .on_drop(move |e: &Dropped, window, app| {
                let mine = drop_kind.as_ref() == Some(&e.item.kind) && own_drop.contains(&e.item.id);
                let at = m.slot_at(e.local, if mine { tiles_now } else { tiles_now + 1 });
                dropped(window, app, &|s| s.gap = None);
                if mine {
                    if let Some(f) = &on_move {
                        f(&TileMoved { id: e.item.id.clone(), index: at }, window, app);
                    }
                } else if let Some(f) = &on_drop {
                    let e = Dropped { slot: Some(DropSlot { parent: None, index: at }), ..e.clone() };
                    f(&e, window, app);
                }
            })
            .children(children)
            .into_any_element()
    }
}

impl IntoElement for TileGrid {
    fn into_any_element(self) -> AnyElement {
        deferred(move |window, app| self.build(window, app))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_fill_the_width_up_to_the_maximum() {
        let m = TileMetrics::new(212., 4, 40., 0.74, 8.);
        assert_eq!(m.columns, 4);
        assert_eq!(m.tile, (47., 35.));
        let narrow = TileMetrics::new(130., 4, 40., 0.74, 8.);
        assert_eq!(narrow.columns, 2, "three would be under 40 wide");
        assert_eq!(TileMetrics::new(300., 3, 40., 0.74, 8.).columns, 3);
        assert_eq!(TileMetrics::new(10., 4, 40., 0.74, 8.).columns, 1);
    }

    #[test]
    fn a_short_last_row_keeps_the_pitch() {
        let m = TileMetrics::new(212., 4, 40., 0.74, 8.);
        assert_eq!(m.origin(5), (m.pitch.0, m.pitch.1));
        assert_eq!(m.height(5), 2. * m.tile.1 + 8.);
        assert_eq!(m.height(0), 0.);
    }

    #[test]
    fn the_slot_under_the_pointer_is_clamped_to_the_tiles() {
        let m = TileMetrics::new(212., 4, 40., 0.74, 8.);
        assert_eq!(m.slot_at(Point { x: 60., y: 10. }, 6), 1);
        assert_eq!(m.slot_at(Point { x: 200., y: 50. }, 6), 5, "past the last tile is the last slot");
        assert_eq!(m.slot_at(Point { x: -5., y: -5. }, 6), 0);
    }
}
