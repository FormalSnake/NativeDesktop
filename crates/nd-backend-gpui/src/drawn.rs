//! Drawn implementations of the `nd-components` intents on GPUI (ARCHITECTURE.md 3.6): the same
//! tree that maps to `NSSplitViewController`, `NSToolbar` and `NSGlassEffectView` on AppKit
//! renders here as plain GPUI with the window's look. The Fluent and Linux skins refine these;
//! until then every intent lays its children out and reports its events.

use std::any::TypeId;

use nd_components::{
    Form, FormRow, FormRowProps, FormSection, FormSectionProps, Glass, GlassGroup, GlassProps, Pane, PaneProps,
    PaneRole, Scaffold, SeparatorKind, SeparatorProps, SourceItem, SourceListKind, SourceListProps, SourceSelected,
    SystemItem, SystemItemProps, TabSelected, Titlebar, TitlebarInset, TitlebarProps, Toolbar, ToolbarItem,
    ToolbarSystemItem, ToolbarTab, ToolbarTabProps,
};
use nd_core::NodeId;
use nd_gpui::gpui::{
    self, AnyElement, ElementId, FontWeight, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, Refineable, StatefulInteractiveElement, StyleRefinement, Styled, WindowControlArea, div, px,
};
use nd_ui::backend::WidgetKind;
use nd_ui::with_titlebar_insets;

use crate::backend::Node;
use crate::render::Build;

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
        if let Some(control) = self.form_control(id, kind, node, window, cx) {
            return Some(control);
        }
        let look = self.look;
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
            if p.collapsed && p.role != PaneRole::Content {
                return Some(div().into_any_element());
            }
            let pane = styled(&p.style);
            let pane = match p.role {
                PaneRole::Sidebar => pane.border_r_1().border_color(look.border).bg(look.control),
                PaneRole::Inspector => pane.border_l_1().border_color(look.border).bg(look.control),
                PaneRole::Content | PaneRole::Overlay => pane,
            };
            pane.children(children(window, cx)).into_any_element()
        } else if is::<Toolbar>(kind) || is::<Titlebar>(kind) {
            // The window's title row: it moves the window and keeps clear of the OS controls.
            let base = props::<TitlebarProps>(node).map(|p| p.style.clone()).unwrap_or_default();
            let style = with_titlebar_insets(&base, self.insets);
            let mut row = div().id(ElementId::Integer(id.0));
            row.style().refine(&style);
            row.flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .w_full()
                .flex_none()
                .border_b_1()
                .border_color(look.border)
                .window_control_area(WindowControlArea::Drag)
                .on_mouse_down(MouseButton::Left, |event: &MouseDownEvent, window, _| {
                    if event.click_count >= 2 {
                        window.zoom_window();
                    } else {
                        window.start_window_move();
                    }
                })
                .children(children(window, cx))
                .into_any_element()
        } else if is::<ToolbarItem>(kind) {
            div().flex().occlude().children(children(window, cx)).into_any_element()
        } else if is::<ToolbarSystemItem>(kind) {
            match props::<SystemItemProps>(node)?.item {
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
            div()
                .id(ElementId::Integer(id.0))
                .px_2()
                .py_1()
                .rounded(px(6.))
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
            let mut glass = styled(&p.style).border_1().border_color(look.border);
            glass = glass.bg(p.tint.unwrap_or(look.control));
            glass.children(children(window, cx)).into_any_element()
        } else if is::<GlassGroup>(kind) {
            div().flex().children(children(window, cx)).into_any_element()
        } else if is::<SeparatorKind>(kind) {
            let p = props::<SeparatorProps>(node)?;
            let line = div().bg(look.border);
            if p.vertical { line.w(px(1.)).h_full() } else { line.h(px(1.)).w_full() }.into_any_element()
        } else if is::<SourceListKind>(kind) {
            let p = props::<SourceListProps>(node)?;
            let emit = self.backend.state.borrow().events.emitter(id);
            let mut list = styled(&p.style).flex().flex_col().gap_1().p_2();
            for section in &p.sections {
                if let Some(title) = &section.title {
                    list = list
                        .child(div().pt_2().px_2().text_xs().font_weight(FontWeight::SEMIBOLD).child(title.clone()));
                }
                for item in flatten(&section.items) {
                    let emit = emit.clone();
                    let wake = self.wake.clone();
                    let selected = p.selected.as_ref() == Some(&item.id);
                    let key = item.id.clone();
                    list = list.child(
                        div()
                            .id(ElementId::Name(item.id.clone()))
                            .flex()
                            .flex_row()
                            .px_2()
                            .py_1()
                            .rounded(px(6.))
                            .when_bg(selected, look.control_active)
                            .hover(move |s| s.bg(look.control_hover))
                            .child(div().flex_1().child(item.title.clone()))
                            .children(item.badge.clone())
                            .on_click(move |_, _, cx| {
                                emit.emit(SourceSelected { id: key.clone() });
                                wake(cx);
                            }),
                    );
                }
            }
            list.into_any_element()
        } else if is::<Form>(kind) {
            div().size_full().flex().flex_col().gap_4().p_4().children(children(window, cx)).into_any_element()
        } else if is::<FormSection>(kind) {
            let p = props::<FormSectionProps>(node)?;
            div()
                .flex()
                .flex_col()
                .gap_1()
                .children(p.title.clone().map(|t| div().font_weight(FontWeight::SEMIBOLD).child(t)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(look.border)
                        .bg(look.control)
                        .children(children(window, cx)),
                )
                .children(p.footer.clone().map(|t| div().text_sm().child(t)))
                .into_any_element()
        } else if is::<FormRow>(kind) {
            let p = props::<FormRowProps>(node)?;
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(p.label.clone())
                        .children(p.help.clone().map(|h| div().text_sm().child(h))),
                )
                .children(children(window, cx))
                .into_any_element()
        } else {
            return None;
        };
        Some(element)
    }
}

fn is_kind<W: 'static>(node: &Node) -> bool {
    matches!(node.kind, nd_ui::backend::NodeKind::Widget(w) if is::<W>(w))
}

fn flatten(items: &[SourceItem]) -> Vec<&SourceItem> {
    let mut out = Vec::new();
    for item in items {
        out.push(item);
        out.extend(flatten(&item.children));
    }
    out
}

/// A background only when `on`, for selected rows and segments.
pub(crate) trait WhenBg: Styled + Sized {
    fn when_bg(self, on: bool, color: gpui::Hsla) -> Self {
        if on { self.bg(color) } else { self }
    }
}

impl<T: Styled> WhenBg for T {}
