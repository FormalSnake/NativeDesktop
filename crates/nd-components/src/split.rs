//! `split(id, axis)`: two panes with a divider the user drags. On macOS an `NSSplitView` (thin
//! divider, the system's cursor and drag), its panes layout boundaries the split view sizes; a
//! drawn divider on GPUI backends. The ratio is controlled: a drag reports [`SplitResized`], and
//! feeding the reported ratio back sets nothing (ARCHITECTURE.md 3.13), so a pane model that
//! returns the same value for an unchanged ratio (`panes::set_pane_ratio`) never loops.

use nd_ui::prelude::*;
use nd_ui::semantics::{Response, SemanticAction, Semantics, WidgetSemantics};
use nd_ui::{ContainerElement, DispatchProps, InteractiveElement, Key, Role, StyleRefinement};

pub use crate::panes::Axis;

/// The ratio a split opens at when the app gives none.
pub const DEFAULT_RATIO: f32 = 0.5;

#[derive(Clone, Debug, PartialEq)]
pub struct SplitProps {
    /// `Horizontal` puts the panes side by side, the first on the leading side.
    pub axis: Axis,
    /// The first pane's share of the room the divider leaves, 0 to 1.
    pub ratio: f32,
    pub label: Option<SharedString>,
    /// The split's own style, for backends that draw it.
    pub style: StyleRefinement,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SplitPaneProps {
    /// Smallest size along the split's axis, in points; a drag stops there.
    pub min: Option<f32>,
    pub max: Option<f32>,
    pub style: StyleRefinement,
}

/// The user moved the divider: the first pane's new share.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitResized {
    pub ratio: f32,
}

/// Widget kind of the split container.
pub struct SplitKind;
/// Widget kind of each of its two panes.
pub struct SplitPaneKind;

impl WidgetSemantics for SplitProps {
    fn semantics(&self) -> Semantics {
        let name = self.label.clone().unwrap_or_default();
        Semantics::new(Role::SplitGroup, name.to_string()).value(format!("{:.3}", self.ratio))
    }

    fn respond(&self, action: &SemanticAction) -> Option<Response> {
        let SemanticAction::SetValue(value) = action else { return None };
        Some(match value.trim().trim_end_matches('%').parse::<f32>() {
            Ok(v) => {
                let ratio = if value.trim().ends_with('%') || v > 1. { v / 100. } else { v };
                Ok(Box::new(SplitResized { ratio: ratio.clamp(0., 1.) }))
            }
            Err(_) => crate::forms::not_taken(&format!("\"{value}\" is not a ratio")),
        })
    }
}

pub struct Split {
    root: ContainerElement<SplitProps>,
    first: Option<SplitPane>,
    second: Option<SplitPane>,
}

/// A split of two panes along `axis`. `id` keys it, so the panes keep their content when a
/// sibling appears or goes.
pub fn split(id: impl Into<SharedString>, axis: Axis) -> Split {
    let props = SplitProps { axis, ratio: DEFAULT_RATIO, label: None, style: StyleRefinement::default() };
    let root = ContainerElement::described::<SplitKind>(Key::Id(id.into()), props);
    let root = match axis {
        Axis::Horizontal => root.flex().flex_row(),
        Axis::Vertical => root.flex().flex_col(),
    };
    Split { root, first: None, second: None }
}

impl Split {
    pub fn ratio(mut self, ratio: f32) -> Self {
        self.root.props_mut().ratio = ratio.clamp(0., 1.);
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.root.props_mut().label = Some(label.into());
        self
    }

    /// The leading (or top) pane's content.
    pub fn first(self, content: impl IntoElement) -> Self {
        self.first_pane(split_pane().child(content))
    }

    pub fn second(self, content: impl IntoElement) -> Self {
        self.second_pane(split_pane().child(content))
    }

    /// The leading pane, for one with a minimum or maximum size.
    pub fn first_pane(mut self, pane: SplitPane) -> Self {
        self.first = Some(pane);
        self
    }

    pub fn second_pane(mut self, pane: SplitPane) -> Self {
        self.second = Some(pane);
        self
    }

    pub fn on_resize(mut self, f: impl Fn(&SplitResized, &mut Window, &mut App) + 'static) -> Self {
        self.root = self.root.on(f);
        self
    }
}

impl Styled for Split {
    fn style(&mut self) -> &mut StyleRefinement {
        self.root.style()
    }
}

impl FluentBuilder for Split {}

impl InteractiveElement for Split {
    fn dispatch_props(&mut self) -> &mut DispatchProps {
        self.root.dispatch_props()
    }
}

impl IntoElement for Split {
    fn into_any_element(mut self) -> AnyElement {
        let style = self.root.style().clone();
        self.root.props_mut().style = style;
        let (axis, ratio) = (self.root.props().axis, self.root.props().ratio);
        let ratio = ratio.clamp(0., 1.);
        // The panes' share as flex weights: retained backends whose split view frames the panes
        // replace these, GPUI lays out with them.
        let panes = [(self.first, "first", ratio), (self.second, "second", 1. - ratio)];
        let children = panes.into_iter().map(|(pane, key, share)| {
            let mut pane = pane.unwrap_or_else(split_pane);
            *pane.0.key_mut() = Key::Id(key.into());
            let pane = pane.0.flex_grow(share.max(0.0001)).flex_shrink(1.).flex_basis(relative(0.)).min_w_0().min_h_0();
            let pane = match axis {
                Axis::Horizontal => pane.h_full(),
                Axis::Vertical => pane.w_full(),
            };
            SplitPane(pane)
        });
        self.root.children(children).into_any_element()
    }
}

/// One pane of a [`split`]: a column holding its children.
pub struct SplitPane(ContainerElement<SplitPaneProps>);

pub fn split_pane() -> SplitPane {
    SplitPane(
        ContainerElement::new::<SplitPaneKind>(Key::Id("pane".into()), SplitPaneProps::default()).flex().flex_col(),
    )
}

impl SplitPane {
    /// The pane never shrinks below `min` points along the split's axis.
    pub fn min_size(mut self, min: Pixels) -> Self {
        self.0.props_mut().min = Some(min.as_f32());
        self
    }

    pub fn max_size(mut self, max: Pixels) -> Self {
        self.0.props_mut().max = Some(max.as_f32());
        self
    }
}

impl Styled for SplitPane {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl ParentElement for SplitPane {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.0.extend(elements);
    }
}

impl FluentBuilder for SplitPane {}

impl IntoElement for SplitPane {
    fn into_any_element(mut self) -> AnyElement {
        let style = self.0.style().clone();
        self.0.props_mut().style = style;
        self.0.into_any_element()
    }
}
