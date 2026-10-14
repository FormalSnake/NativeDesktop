//! Drawn implementations of the `nd-components` intents on GPUI (ARCHITECTURE.md 3.6): the same
//! tree that maps to `NSSplitViewController`, `NSToolbar` and `NSGlassEffectView` on AppKit
//! renders here as plain GPUI with the window's look. The Fluent and Linux skins refine these;
//! until then every intent lays its children out and reports its events.

use std::any::TypeId;

use nd_components::{
    Form, FormRow, FormRowProps, FormSection, FormSectionProps, Glass, GlassGroup, GlassProps, Pane, PaneProps,
    PaneRole, Scaffold, SeparatorKind, SeparatorProps, SystemItem, SystemItemProps, TabSelected, Titlebar,
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
            if p.role != PaneRole::Content && (p.collapsed || self.sidebar_hidden(node)) {
                return Some(div().into_any_element());
            }
            let pane = styled(&p.style);
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
            *row.style() = with_titlebar_insets(&merged, self.insets);
            let mut items = children(window, cx);
            if let Some((at, title)) = self.toolbar_title(id, node) {
                items.insert(at, title);
            }
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
            div().flex().occlude().children(children(window, cx)).into_any_element()
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
            // than drawing over what follows.
            let s = &theme.spacing;
            div()
                .id(ElementId::Integer(id.0))
                .size_full()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(s.xl)
                .p(s.md)
                .children(children(window, cx))
                .into_any_element()
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
        parent.children.iter().filter_map(|c| self.nodes.get(c)).any(|n| {
            props::<PaneProps>(n).is_some_and(|p| p.role == PaneRole::Sidebar && !p.collapsed)
                && !self.sidebar_hidden(n)
        })
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
            .on_click(move |_, window, _| {
                if let Some(scaffold) = scaffold {
                    let mut hidden = hidden.borrow_mut();
                    if !hidden.remove(&scaffold) {
                        hidden.insert(scaffold);
                    }
                }
                window.refresh();
            })
            .child(glyph(Glyph::of(Icon::PanelLeft), px(16.), self.ink_text(), px(1.75)))
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
        let color = self.text_run(id, &title, Run::BODY.weight(FontWeight::BOLD), None, None);
        let label = div().min_w_0().truncate().font_weight(FontWeight::BOLD).text_color(color).child(title);
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
