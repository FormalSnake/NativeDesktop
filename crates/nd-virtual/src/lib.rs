//! Virtualization (ARCHITECTURE.md 7): one list, table, grid and tree API over a backend-neutral
//! model. [`Core`] owns the height tree, the anchor, follow mode and jump targets; [`ListState`]
//! holds an app's keys and the row cache; presenters in the backends (an `NSTableView` family on
//! macOS, a GPUI element elsewhere) lay rows out from the model and report what they measure.

mod core;
mod heights;
mod list;
pub mod physics;

#[cfg(feature = "conformance")]
pub mod conformance;

pub use crate::core::{Align, Alignment, Core, Diff, Follow, GridSpec, Layout, Placed, RowId};
pub use crate::list::{
    ActivateEvent, CellTag, Column, ColumnResized, ExpandEvent, List, ListHandle, ListKind, ListProps, ListState,
    ListWidget, RowActivated, RowDisclosed, RowKey, RowTag, RowView, RowsSelected, SelectEvent, Selection, SortChanged,
    SortDirection, TreeNode, ViewportChanged, column, grid, list, table, tree,
};
