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
        if let Some(element) = self.nav(id, kind, node, window) {
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
            div()
                .size_full()
                .flex()
                .flex_col()
                .children(bars.iter().map(|c| self.node(*c, window, cx)).collect::<Vec<_>>())
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_1()
                        .min_h_0()
                        .children(panes.iter().map(|c| self.node(*c, window, cx)).collect::<Vec<_>>()),
                )
                .into_any_element()
        } else if is::<Pane>(kind) {
            let p = props::<PaneProps>(node)?;
            let fluent = theme.preset == nd_theme::Preset::Fluent;
            if p.role != PaneRole::Content && p.collapsed {
                return Some(div().into_any_element());
            }
            // Hidden by the toolbar toggle: WinUI's navigation pane folds to an icon rail,
            // elsewhere the sidebar goes.
            let rail = p.role == PaneRole::Sidebar && self.sidebar_hidden(node);
            if rail && !fluent {
                return Some(div().into_any_element());
            }
            let pane = styled(&p.style);
            let pane = if rail { pane.w(px(48.)).min_w(px(48.)).flex_none() } else { pane };
            let sidebar = theme.surface.sidebar;
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
            self.painted(id, fill, None, "pane", || pane.children(children(window, cx)).into_any_element())
        } else if is::<Toolbar>(kind) || is::<Titlebar>(kind) {
            // The window's title row: it moves the window and keeps clear of the OS controls.
            // Defaults, then the app's style, then the OS insets added on top of both: the
            // insets pad out from whatever padding the row ends up with.
            let s = &theme.spacing;
            let mut row = div().id(ElementId::Integer(id.0)).p(s.sm).min_h(theme.control.regular + s.sm * 2.);
            if let Some(p) = props::<TitlebarProps>(node) {
                row.style().refine(&p.style);
            }
            let merged = row.style().clone();
            // A centred item (Messages' conversation) sits at the middle of the window: the
            // items before and after it go in two groups that share what is left equally, each
            // holding its own OS inset, so the centre stays put on resize and the groups never
            // shrink under their content (the centre gives way instead of overlapping them).
            let centered = node.children.iter().position(|c| {
                self.nodes.get(c).and_then(|n| n.props.downcast_ref::<ToolbarItemProps>()).is_some_and(|p| p.centered)
            });
            let insets = if centered.is_some() {
                nd_ui::TitlebarInsets { leading: 0., trailing: 0., ..self.insets }
            } else {
                self.insets
            };
            *row.style() = with_titlebar_insets(&merged, insets);
            let mut items = children(window, cx);
            let mut centered = centered;
            if let Some((at, title)) = self.toolbar_title(id, node) {
                if let Some(c) = centered.as_mut()
                    && at <= *c
                {
                    *c += 1;
                }
                items.insert(at, title);
            }
            let items = match centered {
                Some(c) => {
                    let after: Vec<AnyElement> = items.split_off(c + 1);
                    let center = items.pop().expect("the centred item");
                    let side = || div().flex().flex_row().items_center().gap(s.sm).flex_1().flex_basis(px(0.));
                    vec![
                        side().pl(px(self.insets.leading)).children(items).into_any_element(),
                        div().flex_none().child(center).into_any_element(),
                        side().justify_end().pr(px(self.insets.trailing)).children(after).into_any_element(),
                    ]
                }
                None => items,
            };
            row.flex()
                .flex_row()
                .items_center()
                .gap(s.sm)
                .w_full()
                .flex_none()
                .when(!self.backdrop_shows(theme.materials.toolbar), |row| row.border_b_1().border_color(look.border))
                .window_control_area(WindowControlArea::Drag)
                .on_mouse_down(MouseButton::Left, |event: &MouseDownEvent, window, _| {
                    if event.click_count >= 2 {
                        window.zoom_window();
                    } else {
                        crate::render::start_window_move(window);
                    }
                })
                .children(items)
                .into_any_element()
        } else if is::<ToolbarItem>(kind) {
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
                (Some(title), true) => div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(title.px(s.md).pt(s.md))
                    .child(rows.flex_1().min_h_0().children(children(window, cx)))
                    .into_any_element(),
                (title, _) => rows.size_full().children(title).children(children(window, cx)).into_any_element(),
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
        let at = node
            .children
            .iter()
            .take_while(|c| {
                matches!(system(c), Some(SystemItem::ToggleSidebar | SystemItem::SidebarTrackingSeparator))
                    || item(c).is_some_and(|p| p.leading)
            })
            .count();
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

fn is_kind<W: 'static>(node: &Node) -> bool {
    matches!(node.kind, nd_ui::backend::NodeKind::Widget(w) if is::<W>(w))
}

/// A background only when `on`, for selected rows and segments.
pub(crate) trait WhenBg: Styled + Sized {
    fn when_bg(self, on: bool, color: gpui::Hsla) -> Self {
        if on { self.bg(color) } else { self }
    }
}

impl<T: Styled> WhenBg for T {}
