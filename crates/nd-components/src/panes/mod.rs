//! Split-pane models, pure and with no platform code, and the elements that render them.
//!
//! - [`model`]: a binary tree of splits and leaves with a focused leaf: split, close, directional
//!   focus, divider ratios, persistence.
//! - [`dock`]: the same tree with tab stacks as leaves: add, close, move and undock tabs, dock a
//!   panel against another, and the drag-and-drop policy that turns a drop into one op.
//! - [`tiles`]: freeform tiles on a column grid with push-down collision handling.
//! - [`pane_tree`], [`dock_view`], [`tiles_view`]: the elements that render them.
//!
//! Every op returns a value equal to its input and sharing its `Rc` when nothing changed, so a
//! native echo of state the model already holds cannot loop a render and persist cycle. Check
//! with `PaneTree::same` / `TileModel::same`.

pub mod dock;
pub mod model;
pub mod tiles;
mod views;

pub use dock::{
    DockDrag, DockDragKind, DockEdgeZone, DockModel, DockPanel, DockRect, DockSize, DockSnapshot, DockTab,
    DockTabLocation, DockZone, activate_tab, active_dock_tab, active_dock_tab_index, add_tab, apply_dock_drop,
    close_panel, close_tab, deserialize_dock, dock_drag_payload, dock_panel, dock_panel_of, dock_panel_rects,
    dock_panels, dock_zone_at, empty_dock, find_dock_panel, find_dock_tab, hit_test_dock_zone, move_tab,
    parse_dock_drag, seed_dock, seed_dock_with, serialize_dock, undock_tab,
};
pub use model::{
    Axis, Direction, Leaf, Node, PaneId, PaneNode, PaneSnapshot, PaneTree, SnapshotNode, clamp_pane_ratio, close_pane,
    empty_panes, find_pane_node, focus_neighbor, focus_pane, focus_pane_at, migrate_panes, migrate_panes_with,
    pane_id_at, pane_leaves, parent_pane_split, same_pane_shape, seed_panes, seed_panes_with, serialize_panes,
    set_pane_ratio, split_pane, update_pane,
};
pub use tiles::{
    Tile, TileCell, TileModel, TilePlacement, TileSize, TileSnapshot, apply_tile_drop, deserialize_tiles, empty_tiles,
    find_tile, move_tile, parse_tile_drag, place_tile, raise_tile, remove_tile, resize_tile, seed_tiles,
    serialize_tiles, tile_cell_at, tile_drag_payload, tile_rows, update_tile,
};
pub use views::{DockView, PaneResized, PaneTreeView, TilesView, dock_view, pane_tree, tiles_view};
