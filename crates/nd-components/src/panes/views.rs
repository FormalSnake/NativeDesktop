//! Elements over the pane models: [`pane_tree`] renders a [`PaneTree`] as nested [`split`]s,
//! [`dock_view`] a dock's panels as tab strips over their tabs, [`tiles_view`] tiles on a column
//! grid. They hold no state: the app keeps the model, renders it, and folds each event back in
//! through the model's ops, whose same-value answer for an echo ends the loop.
//!
//! A leaf's content moves deeper in the element tree when its pane splits; content the app
//! renders as a view (an `Entity`) keeps its native nodes across that move.

use std::rc::Rc;

use nd_ui::prelude::*;

use super::dock::{DockModel, DockTab, active_dock_tab};
use super::model::{Leaf, Node, PaneId, PaneNode, PaneTree, clamp_pane_ratio};
use super::tiles::{Tile, TileModel, tile_rows};
use crate::split::split;
use crate::tabs::{TabChanged, TabCloseRequested, TabStyle, tab_page, tab_view};

/// The user dragged the divider of split `split`.
#[derive(Clone, Debug, PartialEq)]
pub struct PaneResized {
    pub split: PaneId,
    pub ratio: f32,
}

type Handler<E> = Rc<dyn Fn(&E, &mut Window, &mut App)>;
type LeafFn<T> = Rc<dyn Fn(Leaf<'_, T>, bool) -> AnyElement>;
type DrawFn<T> = Rc<dyn Fn(&T) -> AnyElement>;

/// A pane tree as nested splits.
pub struct PaneTreeView<T> {
    id: SharedString,
    tree: PaneTree<T>,
    leaf: LeafFn<T>,
    on_resize: Option<Handler<PaneResized>>,
}

/// Renders `tree`; `leaf` draws each leaf, told whether it is the focused one.
pub fn pane_tree<T: 'static>(
    id: impl Into<SharedString>,
    tree: &PaneTree<T>,
    leaf: impl Fn(Leaf<'_, T>, bool) -> AnyElement + 'static,
) -> PaneTreeView<T> {
    PaneTreeView { id: id.into(), tree: tree.clone(), leaf: Rc::new(leaf), on_resize: None }
}

impl<T: 'static> PaneTreeView<T> {
    /// A divider moved; the app applies it with `set_pane_ratio`.
    pub fn on_resize(mut self, f: impl Fn(&PaneResized, &mut Window, &mut App) + 'static) -> Self {
        self.on_resize = Some(Rc::new(f));
        self
    }
}

fn render_node<T: 'static>(
    node: &PaneNode<T>,
    focused: &PaneId,
    leaf: &LeafFn<T>,
    on_resize: &Option<Handler<PaneResized>>,
) -> AnyElement {
    match &**node {
        Node::Leaf { id, data } => div()
            .id(format!("pane:{id}"))
            .size_full()
            .flex()
            .flex_col()
            .child(leaf(Leaf { id, data }, id == focused))
            .into_any_element(),
        Node::Split { id, axis, ratio, children } => {
            let mut view = split(format!("split:{id}"), *axis)
                .ratio(clamp_pane_ratio(*ratio))
                .size_full()
                .first(render_node(&children[0], focused, leaf, on_resize))
                .second(render_node(&children[1], focused, leaf, on_resize));
            if let Some(handler) = on_resize.clone() {
                let split = id.clone();
                view = view.on_resize(move |e, window, cx| {
                    handler(&PaneResized { split: split.clone(), ratio: clamp_pane_ratio(e.ratio) }, window, cx)
                });
            }
            view.into_any_element()
        }
    }
}

impl<T: 'static> IntoElement for PaneTreeView<T> {
    fn into_any_element(self) -> AnyElement {
        let content =
            self.tree.root().map(|root| render_node(root, self.tree.focused_id(), &self.leaf, &self.on_resize));
        div().id(self.id).size_full().flex().flex_col().children(content).into_any_element()
    }
}

/// A dock: each panel a strip of closable tabs over the active tab's content.
pub struct DockView<T> {
    id: SharedString,
    model: DockModel<T>,
    tab: DrawFn<DockTab<T>>,
    on_activate: Option<Handler<TabChanged>>,
    on_close: Option<Handler<TabCloseRequested>>,
    on_resize: Option<Handler<PaneResized>>,
}

/// Renders `model`; `tab` draws a tab's content. Every tab stays mounted, so a terminal or a
/// page in a background tab keeps running.
pub fn dock_view<T: 'static>(
    id: impl Into<SharedString>,
    model: &DockModel<T>,
    tab: impl Fn(&DockTab<T>) -> AnyElement + 'static,
) -> DockView<T> {
    DockView {
        id: id.into(),
        model: model.clone(),
        tab: Rc::new(tab),
        on_activate: None,
        on_close: None,
        on_resize: None,
    }
}

impl<T: 'static> DockView<T> {
    /// A tab was picked; the app applies it with `activate_tab`.
    pub fn on_activate(mut self, f: impl Fn(&TabChanged, &mut Window, &mut App) + 'static) -> Self {
        self.on_activate = Some(Rc::new(f));
        self
    }

    /// A tab's close was pressed; the app applies it with `close_tab`.
    pub fn on_close(mut self, f: impl Fn(&TabCloseRequested, &mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(f));
        self
    }

    pub fn on_resize(mut self, f: impl Fn(&PaneResized, &mut Window, &mut App) + 'static) -> Self {
        self.on_resize = Some(Rc::new(f));
        self
    }
}

impl<T: 'static> IntoElement for DockView<T> {
    fn into_any_element(self) -> AnyElement {
        let DockView { id, model, tab, on_activate, on_close, on_resize } = self;
        let mut view = pane_tree(id, &model, move |leaf, _| {
            let panel = leaf.data;
            let mut strip = tab_view(format!("dock:{}", leaf.id)).presentation(TabStyle::Strip).size_full();
            if let Some(active) = active_dock_tab(panel) {
                strip = strip.selected(active.id.clone());
            }
            for t in &panel.tabs {
                strip = strip.tab(tab_page(t.id.clone(), t.title.clone()).closable().child(tab(t)));
            }
            if let Some(f) = on_activate.clone() {
                strip = strip.on_select(move |e, w, cx| f(e, w, cx));
            }
            if let Some(f) = on_close.clone() {
                strip = strip.on_close(move |e, w, cx| f(e, w, cx));
            }
            strip.into_any_element()
        });
        if let Some(f) = on_resize {
            view = view.on_resize(move |e, w, cx| f(e, w, cx));
        }
        view.into_any_element()
    }
}

/// Tiles on a grid of `columns` equal columns and rows `row_height` tall.
pub struct TilesView<T> {
    id: SharedString,
    model: TileModel<T>,
    columns: u16,
    row_height: Pixels,
    tile: DrawFn<Tile<T>>,
}

pub fn tiles_view<T: 'static>(
    id: impl Into<SharedString>,
    model: &TileModel<T>,
    row_height: Pixels,
    tile: impl Fn(&Tile<T>) -> AnyElement + 'static,
) -> TilesView<T> {
    let columns = u16::try_from(model.columns().max(1)).unwrap_or(1);
    TilesView { id: id.into(), model: model.clone(), columns, row_height, tile: Rc::new(tile) }
}

impl<T: 'static> IntoElement for TilesView<T> {
    fn into_any_element(self) -> AnyElement {
        let rows = tile_rows(&self.model).max(1);
        let tiles = self.model.tiles().iter().map(|t| {
            div()
                .id(format!("tile:{}", t.id))
                .col_start(i16::try_from(t.x + 1).unwrap_or(1))
                .col_span(u16::try_from(t.w.max(1)).unwrap_or(1))
                .row_start(i16::try_from(t.y + 1).unwrap_or(1))
                .row_span(u16::try_from(t.h.max(1)).unwrap_or(1))
                .min_w_0()
                .min_h_0()
                .flex()
                .flex_col()
                .child((self.tile)(t))
        });
        div()
            .id(self.id)
            .w_full()
            .h(self.row_height * rows as f32)
            .grid()
            .grid_cols(self.columns)
            .grid_rows(u16::try_from(rows).unwrap_or(1))
            .children(tiles)
            .into_any_element()
    }
}
