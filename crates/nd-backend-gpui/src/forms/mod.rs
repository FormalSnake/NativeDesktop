//! Drawn forms controls (`nd_components::forms`) on GPUI, styled from the window's [`Look`]
//! for the Linux look. Each control emits exactly the events its AppKit counterpart does, so
//! the same app code works on both; values stay controlled (the element shows its props, and a
//! user change only emits). What a control keeps between frames (a text field's caret, a
//! slider mid-drag) lives in [`GpuiBackend::drawn`](crate::backend::GpuiBackend), by node.

use std::any::TypeId;

use nd_components::forms::*;
use nd_core::NodeId;
use nd_gpui::gpui::{
    self, AnyElement, ElementId, InteractiveElement, IntoElement, ParentElement, Refineable,
    StatefulInteractiveElement, StyleRefinement, Styled, div, px,
};
use nd_ui::backend::WidgetKind;

use crate::backend::Node;
use crate::drawn::WhenBg;
use crate::render::Build;

fn is<W: 'static>(kind: WidgetKind) -> bool {
    kind.type_id == TypeId::of::<W>()
}

fn styled(style: &StyleRefinement) -> gpui::Div {
    let mut d = div();
    d.style().refine(style);
    d
}

impl Build<'_> {
    /// The drawn control for a forms kind, or `None` when `kind` is not one.
    pub(crate) fn form_control(
        &self,
        id: NodeId,
        kind: WidgetKind,
        node: &Node,
        _window: &mut gpui::Window,
    ) -> Option<AnyElement> {
        let look = self.look;
        let element = if is::<SelectKind>(kind) {
            let p = node.props.downcast_ref::<SelectProps>()?;
            let emit = self.backend.state.borrow().events.emitter(id);
            let mut row = styled(&p.style).flex().flex_row().rounded(px(6.)).border_1().border_color(look.border);
            for (index, option) in p.options.iter().enumerate() {
                let emit = emit.clone();
                let wake = self.wake.clone();
                let selected = index == p.selected;
                row = row.child(
                    div()
                        .id(ElementId::Integer((id.0 << 8) | index as u64))
                        .px_3()
                        .py_1()
                        .when_bg(selected, look.control_active)
                        .hover(move |s| s.bg(look.control_hover))
                        .child(option.clone())
                        .on_click(move |_, _, cx| {
                            emit.emit(SelectChanged { index });
                            wake(cx);
                        }),
                );
            }
            row.into_any_element()
        } else if is::<SliderKind>(kind) {
            let p = node.props.downcast_ref::<SliderProps>()?;
            let span = (p.max - p.min).max(f32::EPSILON);
            let fraction = ((p.value - p.min) / span).clamp(0., 1.);
            styled(&p.style)
                .min_w(px(120.))
                .h(px(6.))
                .rounded_full()
                .bg(look.control)
                .child(div().h_full().rounded_full().bg(look.text).w(gpui::relative(fraction)))
                .into_any_element()
        } else {
            return None;
        };
        Some(element)
    }
}
