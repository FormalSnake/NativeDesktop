//! Drawn implementations of the `nd-components` intents on GPUI (ARCHITECTURE.md 3.6): the same
//! tree that maps to `NSSplitViewController`, `NSToolbar` and `NSGlassEffectView` on AppKit
//! renders here as plain GPUI with the window's look. The Fluent and Linux skins refine these;
//! until then every intent lays its children out and reports its events.

use std::any::TypeId;

use nd_components::{
    Form, FormProps, FormRow, FormRowProps, FormSection, FormSectionProps, Glass, GlassGroup, GlassProps, Pane,
    PaneProps, PaneRole, Scaffold, SeparatorKind, SeparatorProps, SystemItem, SystemItemProps, TabSelected, Titlebar,
    TitlebarInset, TitlebarProps, Toolbar, ToolbarItem, ToolbarItemProps, ToolbarProps, ToolbarSystemItem, ToolbarTab,
    ToolbarTabProps,
};
use nd_components::{REVEAL_EDGE, SidebarRevealed};
use nd_core::NodeId;
use nd_gpui::gpui::prelude::FluentBuilder as _;
use nd_gpui::gpui::{
    self, AnyElement, ElementId, FontWeight, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, Refineable, StatefulInteractiveElement, StyleRefinement, Styled, WindowControlArea, div, px,
};
use nd_theme::preset::Material;
use nd_ui::backend::WidgetKind;
use nd_ui::with_titlebar_insets;

use crate::backend::Node;
use crate::forms::glyph::{Glyph, glyph};
use crate::render::{Build, Run};
use nd_components::icon::Icon;
use nd_components::{FieldStyle, TextChanged, TextFieldKind, TextFieldProps};
use nd_gpui::gpui::SharedString;

fn is<W: 'static>(kind: WidgetKind) -> bool {
    kind.type_id == TypeId::of::<W>()
}

fn styled(style: &StyleRefinement) -> gpui::Div {
    let mut d = div();
    d.style().refine(style);
    d
}

fn props<T: 'static>(node: &Node) -> Option<&T> {
    node.props.downcast_ref::<T>()
}

pub(crate) type PaneSlides = std::rc::Rc<std::cell::RefCell<std::collections::HashMap<NodeId, PaneSlide>>>;

/// A sidebar or inspector opening or shutting: the share shown when it last turned, eased to
/// all or nothing.
pub(crate) struct PaneSlide {
    open: bool,
    from: f32,
    since: nd_core::time::Instant,
    /// Its width when last drawn open, which it slides at.
    width: Option<f32>,
}

impl PaneSlide {
    fn target(&self) -> f32 {
        if self.open { 1. } else { 0. }
    }

    fn shown(&self, now: nd_core::time::Instant, duration: f32) -> f32 {
        let t = (now.duration_since(self.since).as_secs_f32() / duration.max(f32::EPSILON)).min(1.);
        let eased = 1. - (1. - t).powi(3);
        self.from + (self.target() - self.from) * eased
    }
}

impl Build<'_> {
    /// The drawn element for an `nd-components` intent, or `None` when `kind` is none of them.
    pub(crate) fn intent(
        &self,
        id: NodeId,
        kind: WidgetKind,
        node: &Node,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> Option<AnyElement> {
        let children = |w: &mut gpui::Window, cx: &mut gpui::App| -> Vec<AnyElement> {
            node.children.iter().map(|c| self.node(*c, w, cx)).collect()
        };
        if let Some(element) = self.nav(id, kind, node, window, cx) {
            return Some(element);
        }
        if let Some(control) = self.form_control(id, kind, node, window, cx) {
            return Some(control);
        }
        if let Some(media) = self.media(id, kind, node, window, cx) {
            return Some(media);
        }
        let look = self.look();
        let theme = self.theme();
        let ink = self.ink_text();
        let element = if is::<Scaffold>(kind) {
            // Toolbar first as the title row, then the panes side by side.
            let (bars, panes): (Vec<NodeId>, Vec<NodeId>) =
                node.children.iter().partition(|c| self.nodes.get(c).is_some_and(is_kind::<Toolbar>));
            let split = self.split_headers(id, node, &bars);
            let outer = self.split.replace(split);
            let bars: Vec<AnyElement> = bars.iter().map(|c| self.node(*c, window, cx)).collect();
            let panes: Vec<AnyElement> = panes.iter().map(|c| self.node(*c, window, cx)).collect();
            self.split.replace(outer);
            div()
                .size_full()
                .flex()
                .flex_col()
                .children(bars)
                .child(div().flex().flex_row().flex_1().min_h_0().children(panes))
                .into_any_element()
        } else if is::<Pane>(kind) {
            let p = props::<PaneProps>(node)?;
            let fluent = theme.preset == nd_theme::Preset::Fluent;
            if p.role == PaneRole::Sidebar && p.collapsed && p.edge_reveal {
                return Some(self.revealed_sidebar(id, node, p, window, cx));
            }
            // Hidden by the toolbar toggle: WinUI's navigation pane folds to an icon rail,
            // elsewhere the sidebar goes.
            let rail = p.role == PaneRole::Sidebar && self.sidebar_hidden(node);
            let gone = p.role != PaneRole::Content && (p.collapsed || (rail && !fluent));
            let slide = (p.role != PaneRole::Content).then(|| self.pane_slide(id, !gone, &theme)).flatten();
            if gone && slide.is_none() {
                // Its subtree stays in the tree, undrawn: bounds from when it showed would make
                // it look on screen to hit tests and automation.
                self.forget_bounds(node);
                return Some(div().into_any_element());
            }
            let rail = rail && !gone;
            let pane = styled(&p.style);
            let pane = if rail { pane.w(px(48.)).min_w(px(48.)).flex_none() } else { pane };
            let adwaita = theme.preset == nd_theme::Preset::Adwaita;
            let sidebar = match self.inactive(id, window) {
                share if !adwaita || p.role == PaneRole::Content || share <= 0. => theme.surface.sidebar,
                share if share >= 1. => theme.sidebar_backdrop(),
                share => nd_theme::color::mix(theme.surface.sidebar, theme.sidebar_backdrop(), share),
            };
            // Over an OS backdrop (Mica) a see-through sidebar is the bare material, one surface
            // with the title row, and the content beside it sits on an opaque layer whose leading
            // corner takes the window's radius, as Windows' own apps lay it out.
            let clear_sidebar = self.backdrop_shows(theme.materials.sidebar);
            let (pane, fill) = match p.role {
                PaneRole::Sidebar | PaneRole::Inspector if clear_sidebar => (pane, None),
                PaneRole::Sidebar => (pane.border_r_1().border_color(look.border).bg(sidebar), Some(sidebar)),
                PaneRole::Inspector => (pane.border_l_1().border_color(look.border).bg(sidebar), Some(sidebar)),
                PaneRole::Content if clear_sidebar && self.beside_sidebar(node) => {
                    (pane.bg(theme.surface.layer).rounded_tl(theme.radius.lg), Some(theme.surface.layer))
                }
                PaneRole::Content | PaneRole::Overlay => (pane, None),
            };
            let header = self.take_pane_header(node, p.role);
            let pane = self.painted(id, fill, None, "pane", || match header {
                // The pane's own header bar over its body, as in libadwaita's split views; the
                // body keeps the pane's column so a full-height child fills what is left.
                Some(header) => pane
                    .child(header)
                    .child(
                        div().flex_1().min_h_0().w_full().flex().flex_col().relative().children(children(window, cx)),
                    )
                    .into_any_element(),
                None => pane.children(children(window, cx)).into_any_element(),
            });
            match slide {
                // Clipped to the share of its width shown, the pane held at full width against
                // the edge that meets the content, so it slides in from the window's edge.
                Some((shown, width)) => {
                    window.request_animation_frame();
                    let held = div().flex_none().h_full().w(px(width)).flex().child(pane);
                    div()
                        .flex_none()
                        .h_full()
                        .w(px(width * shown))
                        .overflow_hidden()
                        .flex()
                        .when(p.role == PaneRole::Sidebar, |d| d.justify_end())
                        .child(held)
                        .into_any_element()
                }
                None => pane,
            }
        } else if is::<Toolbar>(kind) || is::<Titlebar>(kind) {
            let preset = theme.preset;
            let search = self.toolbar_search(node);
            // WinUI's TitleBar takes a search box as its content, centred and stretched (the
            // Gallery's AutoSuggestBox); libadwaita's header bar shows a toggle for it instead
            // and the search bar under the header (`search_bar`).
            let fill = search.filter(|_| preset == nd_theme::Preset::Fluent);
            self.fill_field.set(fill.map(|(_, field, _)| field));
            let mut items = children(window, cx);
            self.fill_field.set(None);
            // A centred item (Messages' conversation) sits at the middle of its row.
            let mut centered = node
                .children
                .iter()
                .position(|c| {
                    self.nodes
                        .get(c)
                        .and_then(|n| n.props.downcast_ref::<ToolbarItemProps>())
                        .is_some_and(|p| p.centered)
                })
                .map(|c| (c, Center::Item));
            if centered.is_none()
                && let Some((at, _, min)) = fill
            {
                centered = Some((at, Center::Search { min }));
            }
            let over_sidebar = self.over_sidebar(node);
            if let Some((at, title)) = self.toolbar_title(id, node) {
                // libadwaita centres the title widget between the start and end widgets; the
                // start ones run up to the first flexible space.
                let adwaita = preset == nd_theme::Preset::Adwaita && centered.is_none();
                let at = if adwaita {
                    let system = |c: &NodeId| {
                        self.nodes.get(c).and_then(|n| n.props.downcast_ref::<SystemItemProps>()).map(|p| p.item)
                    };
                    node.children[at..]
                        .iter()
                        .position(|c| system(c) == Some(SystemItem::FlexibleSpace))
                        .map_or(items.len(), |p| at + p)
                } else {
                    at
                };
                if let Some((c, _)) = centered.as_mut()
                    && at <= *c
                {
                    *c += 1;
                }
                if adwaita {
                    centered = Some((at, Center::Title));
                }
                items.insert(at, title);
            }
            let bar = search
                .filter(|_| preset == nd_theme::Preset::Adwaita)
                .and_then(|(_, field, _)| self.search_bar(field, window, cx));
            let with_bar = |header: AnyElement, bar: Option<AnyElement>| match bar {
                Some(bar) => div().w_full().flex_none().flex().flex_col().child(header).child(bar).into_any_element(),
                None => header,
            };
            if self.split.borrow().as_ref().is_some_and(|s| node.parent == Some(s.scaffold)) {
                // Split over the panes: what sits over the sidebar heads the sidebar, the title
                // and the rest head the content. The bar itself takes no room.
                let rest = items.split_off(over_sidebar);
                let centered = centered.and_then(|(c, k)| Some((c.checked_sub(over_sidebar)?, k)));
                let mut split = self.split.borrow_mut();
                let split = split.as_mut().expect("split headers");
                let (sidebar, content, inspector) = split.insets(self.insets);
                split.sidebar = Some(self.title_row(
                    id,
                    ElementId::NamedInteger("sidebar".into(), id.0),
                    node,
                    items,
                    None,
                    sidebar,
                    window,
                ));
                let header = self.title_row(id, ElementId::Integer(id.0), node, rest, centered, content, window);
                split.content = Some(with_bar(header, bar));
                if let Some(insets) = inspector {
                    split.inspector = Some(self.title_row(
                        id,
                        ElementId::NamedInteger("inspector".into(), id.0),
                        node,
                        Vec::new(),
                        None,
                        insets,
                        window,
                    ));
                }
                return Some(div().into_any_element());
            }
            let header = self.title_row(id, ElementId::Integer(id.0), node, items, centered, self.insets, window);
            with_bar(header, bar)
        } else if is::<ToolbarItem>(kind)
            && theme.preset == nd_theme::Preset::Adwaita
            && let Some((field, _)) = self.search_field_in(node)
        {
            self.search_toggle(id, field, window)
        } else if is::<ToolbarItem>(kind) {
            let fills = self.fill_field.get().is_some_and(|f| node.children.contains(&f));
            // The item occludes the title row under it, so a press on it is never a window drag
            // (on Windows, never the caption). That also hides it from the window root's pointer
            // listeners, which GPUI only calls where the root is hovered: the item hands its own
            // presses to the gesture router, so a tap on what it holds (Messages' conversation)
            // still fires.
            let (backend, wake, window_id) = (self.backend.clone(), self.wake.clone(), self.window_id);
            let (b2, w2, b3, w3) = (backend.clone(), wake.clone(), backend.clone(), wake.clone());
            let at = |p: gpui::Point<gpui::Pixels>| nd_core::Point { x: p.x.as_f32(), y: p.y.as_f32() };
            div()
                .flex()
                .when(fills, |d| d.w_full())
                .occlude()
                .capture_any_mouse_down(move |e, window, cx| {
                    if e.button == MouseButton::Left {
                        let p = at(e.position);
                        backend.gesture_input(window_id, window, cx, &wake, |g, tree| {
                            g.pointer(window_id, nd_gesture::PointerPhase::Down, p, tree)
                        });
                    }
                })
                .capture_any_mouse_up(move |e, window, cx| {
                    if e.button == MouseButton::Left {
                        let p = at(e.position);
                        b2.gesture_input(window_id, window, cx, &w2, |g, tree| {
                            g.pointer(window_id, nd_gesture::PointerPhase::Up, p, tree)
                        });
                    }
                })
                .on_mouse_move(move |e, window, cx| {
                    if e.pressed_button == Some(MouseButton::Left) {
                        let p = at(e.position);
                        b3.gesture_input(window_id, window, cx, &w3, |g, tree| {
                            g.pointer(window_id, nd_gesture::PointerPhase::Move, p, tree)
                        });
                    }
                })
                .children(children(window, cx))
                .into_any_element()
        } else if is::<ToolbarSystemItem>(kind) {
            match props::<SystemItemProps>(node)?.item {
                SystemItem::ToggleSidebar => self.sidebar_toggle(id, node, &theme),
                SystemItem::FlexibleSpace => div().flex_1().into_any_element(),
                SystemItem::Space => div().w(px(8.)).into_any_element(),
                _ => div().into_any_element(),
            }
        } else if is::<ToolbarTab>(kind) {
            let p = props::<ToolbarTabProps>(node)?;
            let tab = p.id.clone();
            let toolbar = node.parent?;
            let emit = self.backend.state.borrow().events.emitter(toolbar);
            let wake = self.wake.clone();
            let color = self.text_run(id, &p.label, Run::BODY, None, None);
            div()
                .id(ElementId::Integer(id.0))
                .px_2()
                .py_1()
                .rounded(px(6.))
                .text_color(color)
                .hover(move |s| s.bg(look.control_hover))
                .occlude()
                .child(p.label.clone())
                .on_click(move |_, _, cx| {
                    emit.emit(TabSelected { id: tab.clone() });
                    wake(cx);
                })
                .into_any_element()
        } else if is::<TitlebarInset>(kind) {
            div().into_any_element()
        } else if is::<Glass>(kind) {
            let p = props::<GlassProps>(node)?;
            let fill = p.tint.unwrap_or(look.control);
            let glass = styled(&p.style).border_1().border_color(look.border).bg(fill).text_color(ink);
            self.painted(id, Some(fill), None, "glass", || glass.children(children(window, cx)).into_any_element())
        } else if is::<GlassGroup>(kind) {
            div().flex().children(children(window, cx)).into_any_element()
        } else if is::<SeparatorKind>(kind) {
            let p = props::<SeparatorProps>(node)?;
            self.painted(id, Some(look.border), None, "separator", || ());
            let line = div().bg(look.border);
            if p.vertical { line.w(px(1.)).h_full() } else { line.h(px(1.)).w_full() }.into_any_element()
        } else if is::<Form>(kind) {
            // The form scrolls itself, like AppKit's: rows past its height scroll under it rather
            // than drawing over what follows. Its title scrolls with the rows unless pinned.
            let s = &theme.spacing;
            let p = props::<FormProps>(node).cloned().unwrap_or_default();
            let title = p.title.map(|t| {
                // WinUI sets a page's title in TitleLarge (28 px semibold); elsewhere the title style.
                let size = if theme.preset == nd_theme::Preset::Fluent {
                    theme.type_scale.large_title
                } else {
                    theme.type_scale.title
                };
                self.text_run(id, &t, Run::sized(size.as_f32()).weight(FontWeight::SEMIBOLD), None, None);
                div().flex_none().text_size(size).font_weight(FontWeight::SEMIBOLD).child(t)
            });
            let rows = div().id(ElementId::Integer(id.0)).overflow_y_scroll().flex().flex_col().gap(s.xl).p(s.md);
            match (title, p.title_pinned) {
                (Some(title), true) => {
                    let rows = rows.children(children(window, cx));
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .child(title.px(s.md).pt(s.md))
                        .child(self.scrolled(id, rows, crate::scrollbar::Fit::Flex, window))
                        .into_any_element()
                }
                (title, _) => {
                    let rows = rows.children(title).children(children(window, cx));
                    self.scrolled(id, rows, crate::scrollbar::Fit::Fill, window)
                }
            }
        } else if is::<FormSection>(kind) {
            let p = props::<FormSectionProps>(node)?;
            let s = &theme.spacing;
            let secondary = theme.text.secondary;
            let title = p.title.clone().map(|t| {
                self.text_run(id, &t, Run::BODY.weight(FontWeight::SEMIBOLD), Some(secondary), None);
                div().mb(s.md).font_weight(FontWeight::SEMIBOLD).text_color(secondary).child(t)
            });
            let rows = self.painted(id, Some(look.control), None, "form section", || {
                div()
                    .flex()
                    .flex_col()
                    .rounded(theme.radius.lg)
                    .border_1()
                    .border_color(look.border)
                    .bg(look.control)
                    .children(children(window, cx))
            });
            let footer = p.footer.clone().map(|t| {
                self.text_run(id, &t, Run::sized(14.), Some(secondary), None);
                div().mt(s.sm).text_sm().text_color(secondary).child(t)
            });
            div().flex().flex_col().flex_none().children(title).child(rows).children(footer).into_any_element()
        } else if is::<FormRow>(kind) {
            let p = props::<FormRowProps>(node)?;
            let label = self.text_run(id, &p.label, Run::BODY, None, None);
            let help = p.help.clone().map(|h| {
                let color = self.text_run(id, &h, Run::sized(14.), Some(theme.text.secondary), None);
                div().text_sm().text_color(color).child(h)
            });
            let s = &theme.spacing;
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(s.md)
                .min_h(theme.control.row)
                .px(s.md)
                .py(s.sm)
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(div().text_color(label).child(p.label.clone()))
                        .children(help),
                )
                .children(children(window, cx))
                .into_any_element()
        } else {
            return None;
        };
        Some(element)
    }
}

impl Build<'_> {
    /// A collapsed `edge_reveal()` sidebar: a strip along the leading edge that slides the
    /// sidebar in as a floating panel over the content while the pointer is on it or the panel.
    /// Takes no room in the row; both paint above the panes after it.
    fn revealed_sidebar(
        &self,
        id: NodeId,
        node: &Node,
        p: &PaneProps,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> AnyElement {
        use crate::reveal::{INSET, RADIUS};
        let now = std::time::Instant::now();
        let emit = self.backend.state.borrow().events.emitter(id);
        let wake = self.wake.clone();
        let (progress, moving, shown) = {
            let mut reveals = self.backend.reveals.borrow_mut();
            let reveal = reveals.entry(id).or_default();
            let pointer = window.mouse_position();
            if reveal.settle(now, nd_core::Point { x: pointer.x.as_f32(), y: pointer.y.as_f32() }) {
                emit.emit(SidebarRevealed { revealed: false });
                wake(cx);
            }
            (reveal.progress(now), reveal.moving(now), reveal.shown())
        };
        if moving {
            window.request_animation_frame();
        }
        let (reveals, edge_emit, edge_wake) = (self.backend.reveals.clone(), emit.clone(), wake.clone());
        let edge = div()
            .id(ElementId::NamedInteger("nd-reveal-edge".into(), id.0))
            .absolute()
            .top_0()
            .left_0()
            .h_full()
            .w(px(REVEAL_EDGE))
            .on_hover(move |inside: &bool, window, cx| {
                let now = std::time::Instant::now();
                let mut reveals = reveals.borrow_mut();
                let reveal = reveals.entry(id).or_default();
                // Off the strip without reaching the panel hides it as leaving the panel does.
                if !*inside {
                    reveal.pointer_left(Some(now));
                } else if reveal.set(true, now) {
                    edge_emit.emit(SidebarRevealed { revealed: true });
                    edge_wake(cx);
                }
                window.refresh();
            });
        let mut slot = div().relative().flex_none().w(px(0.)).h_full().child(gpui::deferred(edge));
        if progress > 0. || shown {
            let width = p.width.unwrap_or(240.).max(p.min_width.unwrap_or(0.));
            let reduce = cx.reduce_motion();
            let x = if reduce { INSET } else { INSET + (progress - 1.) * (width + INSET) };
            let theme = self.theme();
            let (reveals, painted) = (self.backend.reveals.clone(), self.backend.reveals.clone());
            let content = styled(&p.style)
                .size_full()
                .children(node.children.iter().map(|c| self.node(*c, window, cx)).collect::<Vec<_>>());
            let panel = div()
                .id(ElementId::NamedInteger("nd-reveal-panel".into(), id.0))
                .absolute()
                .top(px(INSET))
                .bottom(px(INSET))
                .left(px(x))
                .w(px(width))
                .rounded(px(RADIUS))
                .overflow_hidden()
                .bg(theme.surface.sidebar)
                .border_1()
                .border_color(self.look().border)
                .shadow(vec![gpui::BoxShadow {
                    color: gpui::black().opacity(0.28),
                    offset: gpui::point(px(0.), px(0.)),
                    blur_radius: px(18.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .opacity(if reduce { progress } else { 1. })
                .occlude()
                .on_hover(move |inside: &bool, window, _| {
                    let left = (!*inside).then(std::time::Instant::now);
                    reveals.borrow_mut().entry(id).or_default().pointer_left(left);
                    window.refresh();
                })
                .child(content)
                .child(
                    gpui::canvas(
                        move |b, _, _| {
                            let area = nd_core::Rect::new(
                                b.origin.x.as_f32(),
                                b.origin.y.as_f32(),
                                b.size.width.as_f32(),
                                b.size.height.as_f32(),
                            );
                            painted.borrow_mut().entry(id).or_default().area = Some(area);
                        },
                        |_, (), _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                );
            slot = slot.child(gpui::deferred(panel).with_priority(1));
        }
        slot.into_any_element()
    }

    /// Whether a surface of `material` shows the window's OS backdrop (Mica) through.
    fn backdrop_shows(&self, material: Material) -> bool {
        material != Material::Opaque && self.hooks.backdrop(self.window_id)
    }

    /// Whether a pane has an open sidebar beside it in its scaffold.
    fn beside_sidebar(&self, node: &Node) -> bool {
        let Some(parent) = node.parent.and_then(|p| self.nodes.get(&p)) else { return false };
        let rails = self.theme().preset == nd_theme::Preset::Fluent;
        parent.children.iter().filter_map(|c| self.nodes.get(c)).any(|n| {
            props::<PaneProps>(n).is_some_and(|p| p.role == PaneRole::Sidebar && !p.collapsed)
                && (rails || !self.sidebar_hidden(n))
        })
    }

    /// Whether `node` sits in a sidebar the toolbar toggle folded to a rail.
    pub(crate) fn in_hidden_sidebar(&self, node: &Node) -> bool {
        let mut at = node.parent;
        while let Some(id) = at {
            let Some(n) = self.nodes.get(&id) else { return false };
            if props::<PaneProps>(n).is_some_and(|p| p.role == PaneRole::Sidebar) {
                return self.sidebar_hidden(n);
            }
            at = n.parent;
        }
        false
    }

    /// Whether the drawn toolbar toggle hid the sidebar of the scaffold `pane` sits in.
    /// While the pane slides open or shut, the share of it shown and its width. `None` once it
    /// rests, and on the first draw: a pane the window opens with is simply there.
    fn pane_slide(&self, id: NodeId, open: bool, theme: &nd_theme::Theme) -> Option<(f32, f32)> {
        let now = nd_core::time::Instant::now();
        let mut slides = self.backend.pane_slides.borrow_mut();
        let slide =
            slides.entry(id).or_insert(PaneSlide { open, from: if open { 1. } else { 0. }, since: now, width: None });
        let duration = theme.motion.normal.as_secs_f32();
        let reduce = self.backend.platform_reduce_motion() == Some(true);
        if slide.open != open {
            slide.from = if reduce { slide.target() } else { slide.shown(now, duration) };
            (slide.open, slide.since) = (open, now);
        }
        if reduce {
            slide.from = slide.target();
        }
        let shown = slide.shown(now, duration);
        if shown == slide.target() {
            // At rest open, the tracked bounds are the pane's own (sliding, they are the clip's).
            if open && let Some(b) = self.backend.bounds(id).filter(|b| b.size.width > 0.) {
                slide.width = Some(b.size.width);
            }
            return None;
        }
        let width = slide.width.or(self.nodes.get(&id).and_then(|n| props::<PaneProps>(n)).and_then(|p| p.width));
        width.map(|w| (shown, w))
    }

    fn forget_bounds(&self, node: &Node) {
        for c in &node.children {
            self.backend.bounds.borrow_mut().remove(c);
            if let Some(n) = self.nodes.get(c) {
                self.forget_bounds(n);
            }
        }
    }

    fn sidebar_hidden(&self, pane: &Node) -> bool {
        pane.parent.is_some_and(|scaffold| self.backend.hidden_sidebars.borrow().contains(&scaffold))
    }

    /// The toolbar's sidebar toggle: shows and hides its scaffold's sidebar, as AppKit's
    /// `toggleSidebar:` collapses the split view without the app's state changing.
    fn sidebar_toggle(&self, id: NodeId, node: &Node, theme: &nd_theme::Theme) -> AnyElement {
        let look = self.look();
        let scaffold = node.parent.and_then(|toolbar| self.nodes.get(&toolbar)).and_then(|toolbar| toolbar.parent);
        let hidden = self.backend.hidden_sidebars.clone();
        let side = theme.control.regular;
        div()
            .id(ElementId::Integer(id.0))
            .flex_none()
            .size(side)
            .flex()
            .items_center()
            .justify_center()
            .rounded(theme.radius.md)
            .hover(move |s| s.bg(look.control_hover))
            .occlude()
            // The title row's own press starts a window move.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, _| {
                if let Some(scaffold) = scaffold {
                    let mut hidden = hidden.borrow_mut();
                    if !hidden.remove(&scaffold) {
                        hidden.insert(scaffold);
                    }
                }
                window.refresh();
            })
            .child(if theme.preset == nd_theme::Preset::Fluent {
                crate::fluent_icons::glyph(crate::fluent_icons::HAMBURGER, px(16.), self.ink_text()).into_any_element()
            } else {
                glyph(Glyph::of(Icon::PanelLeft), px(16.), self.ink_text(), px(1.75)).into_any_element()
            })
            .into_any_element()
    }

    /// A title row: it moves the window and keeps clear of the OS controls by `insets`.
    #[allow(clippy::too_many_arguments)]
    fn title_row(
        &self,
        id: NodeId,
        element_id: ElementId,
        node: &Node,
        items: Vec<AnyElement>,
        centered: Option<(usize, Center)>,
        insets: nd_ui::TitlebarInsets,
        window: &mut gpui::Window,
    ) -> AnyElement {
        let theme = self.theme();
        let look = self.look();
        // Defaults, then the app's style, then the OS insets added on top of both: the insets
        // pad out from whatever padding the row ends up with.
        let s = &theme.spacing;
        // libadwaita's header bar is 47 px whether or not it draws its shade.
        let min_h = match theme.preset {
            nd_theme::Preset::Adwaita => px(47.),
            _ => theme.control.regular + s.sm * 2.,
        };
        let mut row = div().id(element_id).p(s.sm).min_h(min_h);
        if let Some(p) = props::<TitlebarProps>(node) {
            row.style().refine(&p.style);
        }
        let merged = row.style().clone();
        // A centred item sits at the middle of the row: the items before and after it go in two
        // groups that share what is left equally, each holding its own OS inset, so the centre
        // stays put on resize and the groups never shrink under their content (the centre gives
        // way instead of overlapping them).
        let row_insets =
            if centered.is_some() { nd_ui::TitlebarInsets { leading: 0., trailing: 0., ..insets } } else { insets };
        *row.style() = with_titlebar_insets(&merged, row_insets);
        let mut items = items;
        let items = match centered {
            Some((c, kind)) if c < items.len() => {
                let after: Vec<AnyElement> = items.split_off(c + 1);
                let center = items.pop().expect("the centred item");
                let center = match kind {
                    Center::Item => div().flex_none().child(center),
                    // A title gives way first, ellipsized, as a header bar's title label does.
                    Center::Title => div().flex().min_w_0().overflow_hidden().child(center),
                    // WinUI's title bar search: stretched up to the Gallery's 580 px, never
                    // under the width the app gave the field.
                    Center::Search { min } => {
                        div().flex().flex_basis(px(SEARCH_MAX)).max_w(px(SEARCH_MAX)).min_w(px(min)).child(center)
                    }
                };
                let side = || div().flex().flex_row().items_center().gap(s.sm).flex_1().flex_basis(px(0.));
                // The insets are spacers, not padding: a group's padding would add to its flex
                // basis and push the centre off the middle by half the inset.
                // A zero inset adds no spacer: the group's gap would still push its first item in.
                let inset = |w: f32| (w > 0.).then(|| div().flex_none().w(px(w)));
                vec![
                    side().children(inset(insets.leading)).children(items).into_any_element(),
                    center.into_any_element(),
                    side().justify_end().children(after).children(inset(insets.trailing)).into_any_element(),
                ]
            }
            _ => items,
        };
        // libadwaita's title row is a toolbar view's flat top bar, with no shade under it.
        let edge = theme.preset != nd_theme::Preset::Adwaita && !self.backdrop_shows(theme.materials.toolbar);
        // In an inactive window libadwaita draws a title row's content at half opacity, as
        // WinUI draws its deactivated title bar's.
        let content = 1. - 0.5 * self.inactive(id, window);
        let row = row
            .flex()
            .flex_row()
            .items_center()
            .gap(s.sm)
            .w_full()
            .opacity(content)
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, |event: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if event.click_count >= 2 {
                    window.zoom_window();
                } else {
                    crate::render::start_window_move(window);
                }
            })
            .children(items);
        div()
            .w_full()
            .flex_none()
            .when(edge, |d| d.border_b_1().border_color(look.border))
            .child(row)
            .into_any_element()
    }

    /// The search field a toolbar item holds, with its props, when the item is one.
    fn search_field_in(&self, item: &Node) -> Option<(NodeId, &TextFieldProps)> {
        let [field] = item.children.as_slice() else { return None };
        let node = self.nodes.get(field).filter(|n| is_kind::<TextFieldKind>(n))?;
        let props = props::<TextFieldProps>(node).filter(|p| p.field == FieldStyle::Search)?;
        Some((*field, props))
    }

    /// A toolbar's first search field: the item's index, the field and the width the app gave it.
    fn toolbar_search(&self, node: &Node) -> Option<(usize, NodeId, f32)> {
        node.children.iter().enumerate().find_map(|(at, c)| {
            let item = self.nodes.get(c).filter(|n| is_kind::<ToolbarItem>(n))?;
            let (field, props) = self.search_field_in(item)?;
            let width = match props.style.size.width {
                Some(gpui::Length::Definite(gpui::DefiniteLength::Absolute(gpui::AbsoluteLength::Pixels(w)))) => {
                    w.as_f32()
                }
                _ => SEARCH_MIN,
            };
            Some((at, field, width))
        })
    }

    /// Whether a toolbar search field's libadwaita search bar shows: the toggle opened it, or
    /// the field holds text or focus (Ctrl+F moves focus there), which keeps it open.
    fn search_shown(&self, field: NodeId, window: &gpui::Window) -> bool {
        let text = self.nodes.get(&field).and_then(props::<TextFieldProps>).is_some_and(|p| !p.text.is_empty());
        let requested = self.backend.focus_requests.borrow().get(&self.window_id) == Some(&Some(field));
        let focused = self.backend.form_drawn.borrow().get(&field).is_some_and(|d| d.focus.is_focused(window));
        let mut bars = self.backend.search_bars.borrow_mut();
        if text || requested || focused {
            bars.insert(field);
        }
        bars.contains(&field)
    }

    /// Opens or closes a toolbar search field's search bar. Opening focuses the field; closing
    /// clears it and gives up its focus, as `GtkSearchBar` leaves search mode.
    fn search_mode(&self, field: NodeId) -> impl Fn(bool, &mut gpui::Window, &mut gpui::App) + 'static {
        let backend = self.backend.clone();
        let (wake, window_id) = (self.wake.clone(), self.window_id);
        let emit = self.backend.state.borrow().events.emitter(field);
        let text = self.nodes.get(&field).and_then(props::<TextFieldProps>).is_some_and(|p| !p.text.is_empty());
        move |open, window, cx| {
            if open {
                backend.search_bars.borrow_mut().insert(field);
                backend.focus_requests.borrow_mut().insert(window_id, Some(field));
                backend.platform_focus.borrow_mut().push((window_id, Some(field)));
            } else {
                backend.search_bars.borrow_mut().remove(&field);
                let mut requests = backend.focus_requests.borrow_mut();
                if requests.get(&window_id) == Some(&Some(field)) {
                    requests.remove(&window_id);
                }
                drop(requests);
                let focused = backend.form_drawn.borrow().get(&field).is_some_and(|d| d.focus.is_focused(window));
                if focused {
                    window.blur(cx);
                    backend.platform_focus.borrow_mut().push((window_id, None));
                }
                if text {
                    emit.emit(TextChanged { text: SharedString::default() });
                }
            }
            wake(cx);
            window.refresh();
        }
    }

    /// libadwaita's search button: a flat toggle in the header bar, checked while the search
    /// bar shows.
    fn search_toggle(&self, id: NodeId, field: NodeId, window: &mut gpui::Window) -> AnyElement {
        let theme = self.theme();
        let ink = self.ink_text();
        let shown = self.search_shown(field, window);
        // `%button_basic_flat`: 7% hovered, 16% pressed; checked `$selected_color` 10%, 13%
        // hovered, 19% pressed.
        let tint = |share: f32| nd_theme::color::with_alpha(ink, ink.a * share);
        let (rest, hover, active) =
            if shown { (tint(0.10), tint(0.13), tint(0.19)) } else { (gpui::transparent_black(), tint(0.07), tint(0.16)) };
        let mode = self.search_mode(field);
        div()
            .id(ElementId::Integer(id.0))
            .flex_none()
            .size(theme.control.regular)
            .flex()
            .items_center()
            .justify_center()
            .rounded(theme.radius.md)
            .bg(rest)
            .hover(move |s| s.bg(hover))
            .active(move |s| s.bg(active))
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| mode(!shown, window, cx))
            .child(glyph(Glyph::Search, px(16.), ink, px(1.75)))
            .into_any_element()
    }

    /// libadwaita's search bar under the header that holds the toggle: the field centred in
    /// a flat bar, as a toolbar view's top bar draws `GtkSearchBar` (6 px all round, no
    /// shade). Esc leaves search mode. `None` while it is closed.
    fn search_bar(&self, field: NodeId, window: &mut gpui::Window, cx: &mut gpui::App) -> Option<AnyElement> {
        if !self.search_shown(field, window) {
            self.backend.bounds.borrow_mut().remove(&field);
            if let Some(node) = self.nodes.get(&field) {
                self.forget_bounds(node);
            }
            return None;
        }
        let mode = self.search_mode(field);
        Some(
            div()
                .w_full()
                .flex_none()
                .flex()
                .justify_center()
                .p(px(6.))
                .on_key_down(move |e: &gpui::KeyDownEvent, window, cx| {
                    if e.keystroke.key == "escape" {
                        cx.stop_propagation();
                        mode(false, window, cx);
                    }
                })
                .child(self.node(field, window, cx))
                .into_any_element(),
        )
    }

    /// How many of a toolbar's items sit over the sidebar: the sidebar toggle, the tracking
    /// separator and the leading items, the run the title follows.
    fn over_sidebar(&self, node: &Node) -> usize {
        let system =
            |c: &NodeId| self.nodes.get(c).and_then(|n| n.props.downcast_ref::<SystemItemProps>()).map(|p| p.item);
        let item = |c: &NodeId| self.nodes.get(c).and_then(|n| n.props.downcast_ref::<ToolbarItemProps>());
        node.children
            .iter()
            .take_while(|c| {
                matches!(system(c), Some(SystemItem::ToggleSidebar | SystemItem::SidebarTrackingSeparator))
                    || item(c).is_some_and(|p| p.leading)
            })
            .count()
    }

    /// Whether a scaffold's toolbar splits into a header bar per pane, as libadwaita's
    /// `AdwNavigationSplitView` and `AdwOverlaySplitView` lay out Files and Settings: only on
    /// the libadwaita look, with a toolbar and an open sidebar.
    fn split_headers(&self, id: NodeId, node: &Node, bars: &[NodeId]) -> Option<SplitHeaders> {
        if self.theme().preset != nd_theme::Preset::Adwaita || bars.is_empty() {
            return None;
        }
        let shown = |role: PaneRole| {
            node.children.iter().filter_map(|c| self.nodes.get(c)).any(|n| {
                props::<PaneProps>(n).is_some_and(|p| p.role == role && !p.collapsed)
                    && !(role == PaneRole::Sidebar && self.sidebar_hidden(n))
            })
        };
        shown(PaneRole::Sidebar).then(|| SplitHeaders {
            scaffold: id,
            inspector_shown: shown(PaneRole::Inspector),
            sidebar: None,
            content: None,
            inspector: None,
        })
    }

    /// The header bar the split toolbar left for a pane of the scaffold being built.
    fn take_pane_header(&self, node: &Node, role: PaneRole) -> Option<AnyElement> {
        let mut split = self.split.borrow_mut();
        let split = split.as_mut().filter(|s| node.parent == Some(s.scaffold))?;
        match role {
            PaneRole::Sidebar => split.sidebar.take(),
            PaneRole::Content => split.content.take(),
            PaneRole::Inspector => split.inspector.take(),
            PaneRole::Overlay => None,
        }
    }

    /// A toolbar's title and the child index it goes in at: after the sidebar toggle and the
    /// leading items. The toolbar's own `title` shows whatever else it holds (empty hides it);
    /// without one, the window's title fills the leading run when the app put nothing before the
    /// first flexible space.
    fn toolbar_title(&self, id: NodeId, node: &Node) -> Option<(usize, AnyElement)> {
        let props = props::<ToolbarProps>(node)?;
        let system =
            |c: &NodeId| self.nodes.get(c).and_then(|n| n.props.downcast_ref::<SystemItemProps>()).map(|p| p.item);
        let item = |c: &NodeId| self.nodes.get(c).and_then(|n| n.props.downcast_ref::<ToolbarItemProps>());
        let title = match &props.title {
            Some(title) => title.clone(),
            None => {
                let before_space = node.children.iter().take_while(|c| system(c) != Some(SystemItem::FlexibleSpace));
                if before_space.clone().any(|c| item(c).is_some()) {
                    return None;
                }
                self.backend.state.borrow().windows.get(&self.window_id)?.title.clone()
            }
        };
        if title.is_empty() {
            return None;
        }
        let at = self.over_sidebar(node);
        // A WinUI title bar sets the title in the 12 pt Caption style, regular weight.
        let theme = self.theme();
        let fluent = theme.preset == nd_theme::Preset::Fluent;
        let weight = if fluent { FontWeight::NORMAL } else { FontWeight::BOLD };
        let color = self.text_run(id, &title, Run::BODY.weight(weight), None, None);
        let label = div().min_w_0().truncate().font_weight(weight).text_color(color).child(title);
        let label = if fluent { label.text_size(theme.type_scale.caption) } else { label };
        Some((at, label.into_any_element()))
    }
}

/// A scaffold's header bars while its toolbar splits over its panes, each taken by its pane.
pub(crate) struct SplitHeaders {
    scaffold: NodeId,
    inspector_shown: bool,
    sidebar: Option<AnyElement>,
    content: Option<AnyElement>,
    inspector: Option<AnyElement>,
}

impl SplitHeaders {
    /// The OS insets each header keeps clear of: the start controls over the sidebar, the end
    /// ones over the last pane. The inspector's header exists only when the inspector shows.
    fn insets(
        &self,
        window: nd_ui::TitlebarInsets,
    ) -> (nd_ui::TitlebarInsets, nd_ui::TitlebarInsets, Option<nd_ui::TitlebarInsets>) {
        let sidebar = nd_ui::TitlebarInsets { trailing: 0., ..window };
        let middle = nd_ui::TitlebarInsets { leading: 0., ..window };
        if self.inspector_shown {
            (sidebar, nd_ui::TitlebarInsets { trailing: 0., ..middle }, Some(middle))
        } else {
            (sidebar, middle, None)
        }
    }
}

fn is_kind<W: 'static>(node: &Node) -> bool {
    matches!(node.kind, nd_ui::backend::NodeKind::Widget(w) if is::<W>(w))
}

/// What sits at the middle of a title row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Center {
    /// The app's centred item (`centered_item`), at its own size.
    Item,
    /// libadwaita's title widget.
    Title,
    /// WinUI's title bar search box; `min` is the width the app gave the field.
    Search { min: f32 },
}

/// The Gallery's `AutoSuggestBox` `MaxWidth` in the title bar.
const SEARCH_MAX: f32 = 580.;
/// A toolbar search field's least width when the app gave it none.
const SEARCH_MIN: f32 = 200.;
