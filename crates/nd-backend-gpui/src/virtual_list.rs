//! The GPUI presenter of `nd-virtual` lists, tables, grids and trees (ARCHITECTURE.md 7). Our own
//! element, not GPUI's `list`: GPUI's `ListState` trusts heights of rows outside the viewport
//! until `splice` (`gpui/src/elements/list.rs:1-4`), which is where "last rows disappearing"
//! comes from. Here every rendered row is measured at the list's width on every frame before the
//! model places anything, and the model never clamps against an estimate.
//!
//! The rows are the list node's children, rendered by the owner for the model's render range.
//! When the viewport needs rows that are not rendered, the presenter emits `ViewportChanged` and
//! wakes the host; the list's handler renders them in the next app frame, which runs before GPUI
//! draws again.

use nd_core::time::Instant;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::OnceLock;

use nd_core::{NodeId, Rect};
use nd_gpui::gpui::{
    self, AnyElement, App, AvailableSpace, Bounds, ContentMask, DispatchPhase, ElementId, GlobalElementId, Hitbox,
    HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    Pixels, Point, Refineable, Style, fill, point, px, size,
};
use nd_ui::events::Emit;
use nd_virtual::{
    ListHandle, ListKind, ListProps, RowDisclosed, RowId, RowMeta, RowTag, RowsSelected, Selection, ViewportChanged,
};

use crate::backend::Wake;

/// Points of indentation per tree level; the disclosure triangle sits in the last one.
pub(crate) const INDENT: f32 = 16.;
/// Height of a table's header row.
pub(crate) const HEADER: f32 = 24.;

fn now() -> f64 {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// A list's presenter state, kept across frames by the backend.
#[derive(Default)]
pub struct Presenter {
    /// The router's offset for this list when the presenter last synced it with the model, in
    /// router coordinates (`0` is the model's lowest offset). What it moved since is the user's.
    router: Option<f32>,
    /// Rows painted in the last frame: their node and their rectangle in window coordinates.
    pub(crate) painted: Vec<(RowId, NodeId, Rect)>,
    /// The viewport in window coordinates, below a table's header.
    pub(crate) viewport: Rect,
    /// The rendered rows when it last asked the app for others, so it asks once per change.
    asked: Option<Range<usize>>,
    edges: (bool, bool),
}

pub(crate) type Presenters = Rc<RefCell<HashMap<NodeId, Presenter>>>;

pub(crate) struct Row {
    pub tag: RowTag,
    pub node: NodeId,
    pub element: AnyElement,
}

/// A tree row's hierarchy, from the model (the row's own tag can be a render behind).
fn meta(handle: &ListHandle, row: RowId) -> RowMeta {
    let core = handle.0.borrow();
    core.index_of(row).map(|i| core.meta(i)).unwrap_or_default()
}

/// Points a tree row is indented by: its depth plus the disclosure column.
fn tree_indent(kind: &ListKind, handle: &ListHandle, row: RowId) -> f32 {
    if *kind == ListKind::Tree { (f32::from(meta(handle, row).depth) + 1.) * INDENT } else { 0. }
}

pub(crate) struct VirtualList {
    pub node: NodeId,
    pub props: ListProps,
    pub rows: Vec<Row>,
    pub header: Option<AnyElement>,
    pub emit: Emit,
    pub wake: Wake,
    pub presenters: Presenters,
    pub gestures: crate::gestures::Gestures,
    pub window_id: nd_core::WindowId,
    pub selection_color: Hsla,
    /// Under the row the pointer is over, in a list whose rows select or activate.
    pub hover_color: Hsla,
    /// The rounded inset selection of a sidebar, and of every list under WinUI; `None` fills
    /// the row edge to edge, as libadwaita's list views do.
    pub rounded: Option<RoundedSelection>,
    pub disclosure_color: Hsla,
    /// libadwaita's raised and undershoot shades at the scroll edges, under that look.
    pub edges: Option<(Hsla, Hsla)>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RoundedSelection {
    /// From the list's edges.
    pub inset: f32,
    pub radius: f32,
    /// WinUI's selection indicator at the leading edge, in this colour.
    pub pill: Option<Hsla>,
}

pub(crate) struct Prepainted {
    hitbox: Hitbox,
    /// Indices into `rows` that are painted, with their bounds.
    placed: Vec<(usize, Bounds<Pixels>)>,
    selected: Vec<Bounds<Pixels>>,
    disclosures: Vec<(Bounds<Pixels>, bool)>,
    header: Option<Bounds<Pixels>>,
    /// The router's offset and scrollable range this frame.
    scrolled: (f32, f32),
}

impl IntoElement for VirtualList {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

fn rect(b: Bounds<Pixels>) -> Rect {
    Rect::new(b.origin.x.as_f32(), b.origin.y.as_f32(), b.size.width.as_f32(), b.size.height.as_f32())
}

impl VirtualList {
    /// Asks the app for the rows the model wants when they are not the ones rendered.
    fn ask_for_rows(&self, render: Range<usize>, edges: (bool, bool), cx: &mut App) {
        let rendered: HashSet<RowId> = self.rows.iter().map(|r| r.tag.row).collect();
        let core = self.props.handle.0.borrow();
        let wanted: Vec<RowId> = render.clone().filter_map(|i| core.row(i)).collect();
        drop(core);
        let mut presenters = self.presenters.borrow_mut();
        let p = presenters.entry(self.node).or_default();
        let missing = wanted.len() != rendered.len() || wanted.iter().any(|r| !rendered.contains(r));
        let edges_changed = p.edges != edges;
        if (missing && p.asked.as_ref() != Some(&render)) || edges_changed {
            p.asked = Some(render);
            p.edges = edges;
            drop(presenters);
            self.emit.emit(ViewportChanged { at_start: edges.0, at_end: edges.1 });
            (self.wake)(cx);
        }
    }
}

impl gpui::Element for VirtualList {
    type RequestLayoutState = ();
    type PrepaintState = Prepainted;

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::Integer(self.node.0))
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut gpui::Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.refine(&self.props.style);
        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut gpui::Window,
        cx: &mut App,
    ) -> Prepainted {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let header_h = if self.header.is_some() { px(HEADER) } else { px(0.) };
        let header = self.header.as_mut().map(|h| {
            let hb = Bounds::new(bounds.origin, size(bounds.size.width, header_h));
            h.layout_as_root(
                size(AvailableSpace::Definite(hb.size.width), AvailableSpace::Definite(header_h)),
                window,
                cx,
            );
            h.prepaint_at(hb.origin, window, cx);
            hb
        });
        let view = Bounds::new(
            point(bounds.origin.x, bounds.origin.y + header_h),
            size(bounds.size.width, (bounds.size.height - header_h).max(px(0.))),
        );
        let width = view.size.width.as_f32();
        let handle = self.props.handle.clone();
        let grid = handle.0.borrow().grid();

        let viewport = nd_core::Size { width, height: view.size.height.as_f32() };
        {
            let mut core = handle.0.borrow_mut();
            core.set_viewport(f64::from(width), f64::from(viewport.height));
            core.set_time(now());
        }
        // The window's router owns scroll motion (momentum, rubber band, nested arbitration); the
        // model owns position. What the router moved since the last frame goes to the model as a
        // user scroll.
        let routed = self.gestures.list_offset(self.window_id, self.node);
        let last = self.presenters.borrow().get(&self.node).and_then(|p| p.router);
        if let (Some(routed), Some(last)) = (routed, last)
            && routed != last
        {
            handle.0.borrow_mut().scroll_by(f64::from(routed - last));
        }
        // Rows are laid out only where it matters: in the viewport, or with a height the model
        // has not measured at this width. Overscan rows already measured are only kept ready.
        let kind = self.props.kind.clone();
        let by_row: HashMap<RowId, usize> = self.rows.iter().enumerate().map(|(i, r)| (r.tag.row, i)).collect();
        let mut laid: HashSet<usize> = HashSet::new();
        for _ in 0..4 {
            let need: Vec<usize> = {
                let core = handle.0.borrow();
                let visible: HashSet<RowId> = core.visible().iter().map(|p| p.row).collect();
                self.rows
                    .iter()
                    .enumerate()
                    .filter(|(i, r)| !laid.contains(i) && (visible.contains(&r.tag.row) || !core.is_fresh(r.tag.row)))
                    .map(|(i, _)| i)
                    .collect()
            };
            if need.is_empty() {
                break;
            }
            for i in need {
                let row = &mut self.rows[i];
                let indent = tree_indent(&kind, &handle, row.tag.row);
                let available = match grid {
                    Some(g) => size(
                        AvailableSpace::Definite(px(g.item_width as f32)),
                        AvailableSpace::Definite(px(g.item_height as f32)),
                    ),
                    None => size(AvailableSpace::Definite(px((width - indent).max(0.))), AvailableSpace::MinContent),
                };
                let measured = row.element.layout_as_root(available, window, cx);
                handle.0.borrow_mut().report_height(row.tag.row, f64::from(measured.height.as_f32()));
                laid.insert(i);
            }
        }
        let layout = handle.0.borrow_mut().layout();
        // Keep the router on the model's position (rows above changed height, a jump, new rows),
        // without stopping a fling; past an edge the difference is the rubber band, drawn as is.
        let (lo, hi) = handle.0.borrow().bounds();
        let range = (hi - lo) as f32;
        let model = (layout.scroll_top - lo) as f32;
        let synced = self.gestures.sync_list(self.window_id, self.node, &self.props.gestures, viewport, range, model);
        let overscroll = synced - synced.clamp(0., range.max(0.));
        self.presenters.borrow_mut().entry(self.node).or_default().router = Some(synced);
        self.ask_for_rows(layout.render.clone(), (layout.at_start, layout.at_end), cx);
        if handle.0.borrow().animating() || layout.unmeasured {
            window.request_animation_frame();
        }

        let selected: HashSet<RowId> = self.props.selected.iter().copied().collect();
        let mut placed = Vec::new();
        let mut painted = Vec::new();
        let mut sel = Vec::new();
        let mut disclosures = Vec::new();
        window.with_content_mask(Some(ContentMask { bounds: view }), |window| {
            for p in &layout.visible {
                let Some(&i) = by_row.get(&p.row) else { continue };
                let tag = meta(&handle, p.row);
                let indent = tree_indent(&kind, &handle, p.row);
                if !laid.contains(&i) {
                    let available =
                        size(AvailableSpace::Definite(px((width - indent).max(0.))), AvailableSpace::MinContent);
                    self.rows[i].element.layout_as_root(available, window, cx);
                }
                let x = match grid {
                    Some(g) => p.lane as f32 * (g.item_width + g.gap) as f32,
                    None => indent,
                };
                // Rounded to whole points like the rest of GPUI's layout, so rows tile exactly.
                let origin =
                    point(view.origin.x + px(x), view.origin.y + px((p.y - f64::from(overscroll)).round() as f32));
                let h = match grid {
                    Some(g) => g.item_height as f32,
                    None => p.height as f32,
                };
                let w = match grid {
                    Some(g) => g.item_width as f32,
                    None => width - indent,
                };
                let row_bounds = Bounds::new(origin, size(px(w), px(h)));
                self.rows[i].element.prepaint_at(origin, window, cx);
                let full = Bounds::new(point(view.origin.x, origin.y), size(view.size.width, px(h)));
                if selected.contains(&p.row) {
                    sel.push(if grid.is_some() { row_bounds } else { full });
                }
                if tag.expandable {
                    let d = Bounds::new(point(origin.x - px(INDENT), origin.y), size(px(INDENT), px(h)));
                    disclosures.push((d, tag.expanded));
                }
                placed.push((i, row_bounds));
                painted.push((p.row, self.rows[i].node, rect(row_bounds)));
            }
        });
        let mut presenters = self.presenters.borrow_mut();
        let state = presenters.entry(self.node).or_default();
        state.painted = painted;
        state.viewport = rect(view);
        Prepainted { hitbox, placed, selected: sel, disclosures, header, scrolled: (synced, range) }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepainted: &mut Prepainted,
        window: &mut gpui::Window,
        cx: &mut App,
    ) {
        let emit = self.emit.clone();
        let wake = self.wake.clone();
        let rows: Vec<(RowId, Bounds<Pixels>)> =
            prepainted.placed.iter().map(|(i, b)| (self.rows[*i].tag.row, *b)).collect();
        let disclosures: Vec<(RowId, Bounds<Pixels>, bool)> = prepainted
            .placed
            .iter()
            .map(|(i, b)| (self.rows[*i].tag.row, *b, meta(&self.props.handle, self.rows[*i].tag.row)))
            .filter(|(_, _, m)| m.expandable)
            .map(|(r, b, m)| (r, b, m.expanded))
            .collect();
        let selection = self.props.selection;
        let selected = self.props.selected.clone();
        let tree = self.props.kind == ListKind::Tree;
        let hitbox = prepainted.hitbox.clone();
        // The row under the pointer gets the hover fill, as GTK's navigation lists and boxed
        // tables show it; a move onto another row repaints. A plain list of mixed rows (a page
        // of headers and shelves) leaves hover to the rows themselves.
        let hovers = selection != Selection::None
            && (self.props.inset || matches!(self.props.kind, ListKind::Table(_) | ListKind::Tree));
        let row_at = |rows: &[(RowId, Bounds<Pixels>)], at: Point<Pixels>| {
            rows.iter().find(|(_, b)| b.contains(&at)).map(|(r, b)| (*r, *b))
        };
        let hovered = (hovers && hitbox.is_hovered(window)).then(|| row_at(&rows, window.mouse_position())).flatten();
        if hovers {
            let (rows, hitbox) = (rows.clone(), hitbox.clone());
            let shown = hovered.map(|(r, _)| r);
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let now = hitbox.is_hovered(window).then(|| row_at(&rows, event.position)).flatten().map(|(r, _)| r);
                if now != shown {
                    window.refresh();
                }
            });
        }
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left || !hitbox.is_hovered(window) {
                return;
            }
            let at = event.position;
            if tree
                && let Some((row, _, expanded)) = disclosures.iter().find(|(_, b, _)| {
                    at.y >= b.top() && at.y < b.bottom() && at.x < b.left() && at.x >= b.left() - px(INDENT)
                })
            {
                emit.emit(RowDisclosed { row: *row, expanded: !expanded });
                wake(cx);
                return;
            }
            let Some((row, _)) = rows.iter().find(|(_, b)| b.contains(&at)) else { return };
            if event.click_count >= 2 {
                emit.emit(nd_virtual::RowActivated(*row));
            } else if selection != Selection::None {
                let mut next = vec![*row];
                if selection == Selection::Multiple && event.modifiers.secondary() {
                    next = selected.iter().copied().filter(|r| r != row).collect();
                    if !selected.contains(row) {
                        next.push(*row);
                    }
                }
                emit.emit(RowsSelected(next));
            }
            wake(cx);
        });

        if let (Some(header), Some(_)) = (self.header.as_mut(), prepainted.header) {
            header.paint(window, cx);
        }
        let view = {
            let p = self.presenters.borrow();
            p.get(&self.node).map(|p| p.viewport)
        };
        let view = view.map_or(bounds, |v| {
            Bounds::new(point(px(v.origin.x), px(v.origin.y)), size(px(v.size.width), px(v.size.height)))
        });
        let selected_rows: HashSet<RowId> = self.props.selected.iter().copied().collect();
        window.with_content_mask(Some(ContentMask { bounds: view }), |window| {
            if let Some((_, b)) = hovered.filter(|(r, _)| !selected_rows.contains(r)) {
                let full = Bounds::new(point(view.origin.x, b.origin.y), size(view.size.width, b.size.height));
                if let Some(s) = self.rounded {
                    let inner = Bounds::new(
                        point(full.origin.x + px(s.inset), full.origin.y + px(1.)),
                        size(full.size.width - px(s.inset * 2.), full.size.height - px(2.)),
                    );
                    window.paint_quad(fill(inner, self.hover_color).corner_radii(px(s.radius)));
                } else {
                    window.paint_quad(fill(full, self.hover_color));
                }
            }
            for b in &prepainted.selected {
                if let Some(s) = self.rounded {
                    let inner = Bounds::new(
                        point(b.origin.x + px(s.inset), b.origin.y + px(1.)),
                        size(b.size.width - px(s.inset * 2.), b.size.height - px(2.)),
                    );
                    window.paint_quad(fill(inner, self.selection_color).corner_radii(px(s.radius)));
                    if let Some(pill) = s.pill {
                        let h = px(16.).min(inner.size.height - px(8.));
                        let at = point(inner.origin.x, inner.origin.y + (inner.size.height - h) / 2.);
                        window.paint_quad(fill(Bounds::new(at, size(px(3.), h)), pill).corner_radii(px(1.5)));
                    }
                } else {
                    window.paint_quad(fill(*b, self.selection_color));
                }
            }
            for (b, expanded) in &prepainted.disclosures {
                paint_disclosure(window, *b, *expanded, self.disclosure_color);
            }
            for (i, _) in &prepainted.placed {
                self.rows[*i].element.paint(window, cx);
            }
            if let Some(edges) = self.edges {
                let (offset, max) = prepainted.scrolled;
                crate::scrollbar::paint_adwaita_edges(window, view, edges, offset, max);
            }
        });
    }
}

/// A small filled triangle: pointing right when collapsed, down when expanded.
fn paint_disclosure(window: &mut gpui::Window, b: Bounds<Pixels>, expanded: bool, color: Hsla) {
    let c = point(b.left() + b.size.width / 2., b.top() + px(INDENT / 2. + 2.));
    let s = px(4.);
    let mut path = gpui::Path::new(if expanded { point(c.x - s, c.y - s / 2.) } else { point(c.x - s / 2., c.y - s) });
    if expanded {
        path.line_to(point(c.x + s, c.y - s / 2.));
        path.line_to(point(c.x, c.y + s / 2.));
    } else {
        path.line_to(point(c.x + s / 2., c.y));
        path.line_to(point(c.x - s / 2., c.y + s));
    }
    window.paint_path(path, color);
}

/// A table's header: column titles, clicking a sortable one reports `SortChanged`.
pub(crate) fn header(
    columns: &[nd_virtual::Column],
    look: &crate::render::Look,
    text: gpui::Hsla,
    emit: Emit,
    wake: Wake,
) -> AnyElement {
    use gpui::{InteractiveElement, ParentElement, StatefulInteractiveElement, Styled, div};
    let mut row =
        div().flex().flex_row().h(px(HEADER)).w_full().border_b_1().border_color(look.border).text_color(text);
    for (i, c) in columns.iter().enumerate() {
        let (emit, wake) = (emit.clone(), wake.clone());
        let (id, sortable, sort) = (c.id.clone(), c.sortable, c.sort);
        let arrow = match sort {
            Some(nd_virtual::SortDirection::Ascending) => " \u{25b2}",
            Some(nd_virtual::SortDirection::Descending) => " \u{25bc}",
            None => "",
        };
        row = row.child(
            div()
                .id(ElementId::Integer(i as u64))
                .w(px(c.width))
                .flex_shrink_0()
                .px(px(6.))
                .flex()
                .items_center()
                .overflow_hidden()
                .child(format!("{}{arrow}", c.title))
                .when(sortable, |d| {
                    d.on_click(move |_, _, cx| {
                        let direction = match sort {
                            Some(nd_virtual::SortDirection::Ascending) => nd_virtual::SortDirection::Descending,
                            _ => nd_virtual::SortDirection::Ascending,
                        };
                        emit.emit(nd_virtual::SortChanged { column: id.clone(), direction });
                        wake(cx);
                    })
                }),
        );
    }
    row.into_any_element()
}

use gpui::prelude::FluentBuilder;
