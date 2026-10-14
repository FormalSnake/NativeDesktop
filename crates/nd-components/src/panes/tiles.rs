//! Tiles model: freeform panels on a column grid. A tile's `x`, `y`, `w`, `h` map straight onto
//! the grid widget's column, row, column-span and row-span attached properties.
//!
//! Collision handling is the standard grid-layout one: the tile you just touched keeps the cells
//! it asked for and every tile it overlaps is pushed down until it clears, cascading. There is
//! no upward compaction, so a tile left at row 8 stays at row 8.
//!
//! Same identity contract as [`super::model`]: an op that changes nothing returns a clone of the
//! same `Rc`, and every tile the op did not move keeps its `Rc<Tile>` identity so a re-render
//! diffs down to the tiles that actually moved. Cells are `i32`: out-of-range values clamp, so a
//! tile always sits inside the column count with spans of at least 1.

use std::collections::HashSet;
use std::rc::Rc;

use nd_gpui::gpui::SharedString;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tile<T> {
    pub id: SharedString,
    /// Column of the leading edge, 0-based.
    pub x: i32,
    /// Row of the top edge, 0-based.
    pub y: i32,
    /// Width in columns, at least 1.
    pub w: i32,
    /// Height in rows, at least 1.
    pub h: i32,
    pub data: T,
}

struct Inner<T> {
    columns: i32,
    tiles: Vec<Rc<Tile<T>>>,
    next_id: u64,
}

pub struct TileModel<T>(Rc<Inner<T>>);

impl<T> Clone for TileModel<T> {
    fn clone(&self) -> Self {
        TileModel(Rc::clone(&self.0))
    }
}

impl<T: PartialEq> PartialEq for TileModel<T> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
            || (self.0.columns == other.0.columns && self.0.next_id == other.0.next_id && self.0.tiles == other.0.tiles)
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for TileModel<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TileModel")
            .field("columns", &self.0.columns)
            .field("tiles", &self.0.tiles)
            .field("next_id", &self.0.next_id)
            .finish()
    }
}

impl<T> TileModel<T> {
    /// Column count the model clamps against. The native grid still sizes its columns from the
    /// children, so this bounds placement, not pixels.
    pub fn columns(&self) -> i32 {
        self.0.columns
    }

    /// Layout order, which is also paint order: later tiles draw over earlier ones where cells
    /// overlap. [`raise_tile`] moves one last.
    pub fn tiles(&self) -> &[Rc<Tile<T>>] {
        &self.0.tiles
    }

    pub fn next_id(&self) -> u64 {
        self.0.next_id
    }

    /// True when both handles point at the same allocation: the "nothing changed" test.
    pub fn same(&self, other: &TileModel<T>) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    fn new(columns: i32, tiles: Vec<Rc<Tile<T>>>, next_id: u64) -> Self {
        TileModel(Rc::new(Inner { columns, tiles, next_id }))
    }
}

#[derive(Clone, Debug)]
pub struct TilePlacement<T> {
    pub id: Option<SharedString>,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub data: T,
}

impl<T> TilePlacement<T> {
    pub fn new(x: i32, y: i32, w: i32, h: i32, data: T) -> Self {
        TilePlacement { id: None, x, y, w, h, data }
    }

    pub fn with_id(mut self, id: impl Into<SharedString>) -> Self {
        self.id = Some(id.into());
        self
    }
}

fn clamp_columns(columns: i32) -> i32 {
    columns.max(1)
}

/// `(x, y, w, h)` forced inside the grid: spans at least 1 and at most the column count, the
/// tile kept inside the columns, rows non-negative.
fn clamp_cells(x: i32, y: i32, w: i32, h: i32, columns: i32) -> (i32, i32, i32, i32) {
    let w = w.max(1).min(columns);
    let h = h.max(1);
    let x = x.max(0).min(columns - w);
    let y = y.max(0);
    (x, y, w, h)
}

/// `tile` itself when the clamped cells equal its own, so an unmoved tile keeps its identity.
fn clamped<T: Clone>(tile: &Rc<Tile<T>>, x: i32, y: i32, w: i32, h: i32, columns: i32) -> Rc<Tile<T>> {
    let (x, y, w, h) = clamp_cells(x, y, w, h, columns);
    if (x, y, w, h) == (tile.x, tile.y, tile.w, tile.h) {
        return Rc::clone(tile);
    }
    Rc::new(Tile { x, y, w, h, ..(**tile).clone() })
}

fn overlaps<T>(a: &Tile<T>, b: &Tile<T>) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// Pushes every tile that collides with an already-placed one down past it. `anchor_id` is
/// placed first, so the tile the caller just moved or resized wins its cells. Terminates because
/// each push strictly increases a tile's `y` and heights are at least 1.
fn resolve_overlaps<T: Clone>(tiles: &[Rc<Tile<T>>], anchor_id: &str) -> Vec<Rc<Tile<T>>> {
    let mut order: Vec<usize> = (0..tiles.len()).collect();
    order.sort_by_key(|&i| (tiles[i].y, tiles[i].x, i));
    if let Some(pos) = order.iter().position(|&i| tiles[i].id.as_ref() == anchor_id)
        && pos > 0
    {
        let anchor = order.remove(pos);
        order.insert(0, anchor);
    }

    let mut placed: Vec<Rc<Tile<T>>> = Vec::with_capacity(tiles.len());
    let mut out = tiles.to_vec();
    for index in order {
        let mut current = Rc::clone(&tiles[index]);
        while let Some(hit) = placed.iter().find(|p| overlaps(p, &current)) {
            current = Rc::new(Tile { y: hit.y + hit.h, ..(*current).clone() });
        }
        placed.push(Rc::clone(&current));
        out[index] = current;
    }
    out
}

/// The model, or the one it was given when resolution moved nothing.
fn commit<T>(m: &TileModel<T>, tiles: Vec<Rc<Tile<T>>>, next_id: Option<u64>) -> TileModel<T> {
    let next_id = next_id.unwrap_or(m.next_id());
    let changed = tiles.len() != m.tiles().len() || tiles.iter().zip(m.tiles()).any(|(a, b)| !Rc::ptr_eq(a, b));
    if !changed && next_id == m.next_id() {
        return m.clone();
    }
    TileModel::new(m.columns(), tiles, next_id)
}

fn numeric_id(id: &str) -> u64 {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return 0;
    }
    id.parse().unwrap_or(0)
}

pub fn empty_tiles<T>(columns: i32) -> TileModel<T> {
    TileModel::new(clamp_columns(columns), Vec::new(), 1)
}

/// Places `items` in order, each one dropped at its requested cell and pushed clear of what is
/// already down. Ids default to `"1"` to `"n"`.
pub fn seed_tiles<T: Clone>(items: Vec<TilePlacement<T>>, columns: i32) -> TileModel<T> {
    items.into_iter().fold(empty_tiles(columns), |m, item| place_tile(&m, item))
}

pub fn find_tile<'a, T>(m: &'a TileModel<T>, id: &str) -> Option<&'a Rc<Tile<T>>> {
    m.tiles().iter().find(|t| t.id.as_ref() == id)
}

/// Adds a tile at the cells it asks for, pushing whatever was there down. An id already in the
/// model is rejected (same tree), the way `add_tab` rejects a duplicate tab id.
pub fn place_tile<T: Clone>(m: &TileModel<T>, placement: TilePlacement<T>) -> TileModel<T> {
    let minted = placement.id.is_none();
    let id = placement.id.unwrap_or_else(|| m.next_id().to_string().into());
    if find_tile(m, &id).is_some() {
        return m.clone();
    }
    let (x, y, w, h) = clamp_cells(placement.x, placement.y, placement.w, placement.h, m.columns());
    let tile = Rc::new(Tile { id: id.clone(), x, y, w, h, data: placement.data });
    let mut all = m.tiles().to_vec();
    all.push(tile);
    let tiles = resolve_overlaps(&all, &id);
    let next_id = (m.next_id() + u64::from(minted)).max(numeric_id(&id) + 1);
    commit(m, tiles, Some(next_id))
}

pub fn remove_tile<T>(m: &TileModel<T>, id: &str) -> TileModel<T> {
    if find_tile(m, id).is_none() {
        return m.clone();
    }
    commit(m, m.tiles().iter().filter(|t| t.id.as_ref() != id).cloned().collect(), None)
}

fn replaced<T>(m: &TileModel<T>, id: &str, with: &Rc<Tile<T>>) -> Vec<Rc<Tile<T>>> {
    m.tiles().iter().map(|t| if t.id.as_ref() == id { Rc::clone(with) } else { Rc::clone(t) }).collect()
}

pub fn move_tile<T: Clone>(m: &TileModel<T>, id: &str, x: i32, y: i32) -> TileModel<T> {
    let Some(tile) = find_tile(m, id) else { return m.clone() };
    let moved = clamped(tile, x, y, tile.w, tile.h, m.columns());
    if moved.x == tile.x && moved.y == tile.y {
        return m.clone();
    }
    commit(m, resolve_overlaps(&replaced(m, id, &moved), id), None)
}

pub fn resize_tile<T: Clone>(m: &TileModel<T>, id: &str, w: i32, h: i32) -> TileModel<T> {
    let Some(tile) = find_tile(m, id) else { return m.clone() };
    let sized = clamped(tile, tile.x, tile.y, w, h, m.columns());
    if sized.w == tile.w && sized.h == tile.h && sized.x == tile.x {
        return m.clone();
    }
    commit(m, resolve_overlaps(&replaced(m, id, &sized), id), None)
}

/// Moves a tile last in layout order, which is the top of the paint order.
pub fn raise_tile<T>(m: &TileModel<T>, id: &str) -> TileModel<T> {
    let Some(index) = m.tiles().iter().position(|t| t.id.as_ref() == id) else { return m.clone() };
    if index == m.tiles().len() - 1 {
        return m.clone();
    }
    let mut tiles = m.tiles().to_vec();
    let tile = tiles.remove(index);
    tiles.push(tile);
    TileModel::new(m.columns(), tiles, m.next_id())
}

/// Replaces a tile's data with `f(current)`. Equal data is a same-tree no-op.
pub fn update_tile<T: Clone + PartialEq>(m: &TileModel<T>, id: &str, f: impl FnOnce(&T) -> T) -> TileModel<T> {
    let Some(tile) = find_tile(m, id) else { return m.clone() };
    let data = f(&tile.data);
    if data == tile.data {
        return m.clone();
    }
    let updated = Rc::new(Tile { data, ..(**tile).clone() });
    commit(m, replaced(m, id, &updated), None)
}

/// Pixel extent of the grid the tiles are laid out in. A drop reports a point in the grid's own
/// coordinate space and the native grid never reports its geometry, so this is what turns that
/// point back into a cell.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TileSize {
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileCell {
    pub column: i32,
    pub row: i32,
}

const DRAG_PREFIX: &str = "nd-tiles:tile:";

/// The drag payload a tile carries, namespaced for the same reason as `dock_drag_payload`: text
/// dragged in from another application arrives on the same drop handler.
pub fn tile_drag_payload(id: &str) -> String {
    format!("{DRAG_PREFIX}{id}")
}

pub fn parse_tile_drag(payload: &str) -> Option<&str> {
    payload.strip_prefix(DRAG_PREFIX).filter(|id| !id.is_empty())
}

/// Row count the layout currently occupies, at least 1.
pub fn tile_rows<T>(m: &TileModel<T>) -> i32 {
    m.tiles().iter().map(|t| t.y + t.h).fold(1, i32::max)
}

/// Cell under a point in the grid's coordinate space. Cells are taken as uniform, which is what
/// the grid draws when every tile expands. Points outside the grid clamp to its edge cells.
pub fn tile_cell_at<T>(m: &TileModel<T>, size: TileSize, x: f32, y: f32) -> TileCell {
    let columns = m.columns().max(1);
    let rows = tile_rows(m);
    let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
    let (width, height, px, py) = (finite(size.width), finite(size.height), finite(x), finite(y));
    let column = if width > 0.0 { (px * columns as f32 / width).floor() as i32 } else { 0 };
    let row = if height > 0.0 { (py * rows as f32 / height).floor() as i32 } else { 0 };
    TileCell { column: column.clamp(0, columns - 1), row: row.max(0) }
}

/// Turns one drop into one model op: the cell under the pointer becomes the dragged tile's
/// top-left, and [`move_tile`] pushes whatever it lands on clear. A tile dropped back on the cell
/// it already occupies returns the same tree, as does a payload from anything but a tile.
pub fn apply_tile_drop<T: Clone>(m: &TileModel<T>, payload: &str, size: TileSize, x: f32, y: f32) -> TileModel<T> {
    let Some(id) = parse_tile_drag(payload) else { return m.clone() };
    if find_tile(m, id).is_none() {
        return m.clone();
    }
    let cell = tile_cell_at(m, size, x, y);
    move_tile(m, id, cell.column, cell.row)
}

/// Plain-data form of a [`TileModel`], as persisted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TileSnapshot<T> {
    #[serde(default)]
    pub columns: Option<i32>,
    pub tiles: Vec<Tile<T>>,
    #[serde(default)]
    pub next_id: u64,
}

pub fn serialize_tiles<T: Clone>(m: &TileModel<T>) -> TileSnapshot<T> {
    TileSnapshot {
        columns: Some(m.columns()),
        tiles: m.tiles().iter().map(|t| (**t).clone()).collect(),
        next_id: m.next_id(),
    }
}

/// Rebuilds a tile layout from persisted data. An empty or duplicate id, or data failing
/// `is_data`, rejects the whole layout back to `empty_tiles(columns)`; `columns` is also the
/// fallback when the persisted column count is absent. Loaded tiles are clamped and pushed clear
/// of each other, so a hand-edited file cannot land an overlapping layout.
pub fn deserialize_tiles<T: Clone>(raw: TileSnapshot<T>, columns: i32, is_data: impl Fn(&T) -> bool) -> TileModel<T> {
    let fallback = empty_tiles(columns);
    let width = raw.columns.map_or(fallback.columns(), clamp_columns);
    let mut seen: HashSet<SharedString> = HashSet::new();
    let mut tiles: Vec<Rc<Tile<T>>> = Vec::with_capacity(raw.tiles.len());
    let mut max_numeric = 0;
    for tile in raw.tiles {
        if tile.id.is_empty() || !seen.insert(tile.id.clone()) || !is_data(&tile.data) {
            return fallback;
        }
        max_numeric = max_numeric.max(numeric_id(&tile.id));
        let (x, y, w, h) = clamp_cells(tile.x, tile.y, tile.w, tile.h, width);
        tiles.push(Rc::new(Tile { x, y, w, h, ..tile }));
    }
    let tiles = match tiles.first() {
        Some(first) => resolve_overlaps(&tiles, &first.id.clone()),
        None => tiles,
    };
    TileModel::new(width, tiles, raw.next_id.max(max_numeric + 1).max(1))
}
