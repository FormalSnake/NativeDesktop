//! `source_list()`: the sidebar's navigation list, data-driven. On macOS an `NSOutlineView` in
//! the source list style (system selection, group rows, disclosure, row metrics); on GPUI
//! backends the skin's navigation list. Controlled: `selected` is the app's, a click reports
//! [`SourceSelected`]. Expansion of nested items is the outline's own.

use nd_ui::prelude::*;
use nd_ui::{Key, NativeElement, StyleRefinement};

use crate::icon::Icon;

#[derive(Clone, Debug, PartialEq)]
pub struct SourceItem {
    pub id: SharedString,
    pub title: SharedString,
    pub icon: Option<Icon>,
    pub badge: Option<SharedString>,
    pub children: Vec<SourceItem>,
}

/// One row: an id the app selects by, a title, and optionally an icon, a badge and children.
pub fn source_item(id: impl Into<SharedString>, title: impl Into<SharedString>) -> SourceItem {
    SourceItem { id: id.into(), title: title.into(), icon: None, badge: None, children: Vec::new() }
}

impl SourceItem {
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn badge(mut self, badge: impl Into<SharedString>) -> Self {
        self.badge = Some(badge.into());
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = SourceItem>) -> Self {
        self.children = children.into_iter().collect();
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceSection {
    /// A group row; `None` lists the items without one.
    pub title: Option<SharedString>,
    pub items: Vec<SourceItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceListProps {
    pub sections: Vec<SourceSection>,
    pub selected: Option<SharedString>,
    pub style: StyleRefinement,
}

/// The user selected the row with this id.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceSelected {
    pub id: SharedString,
}

/// Widget kind of `source_list`.
pub struct SourceListKind;

pub struct SourceList(NativeElement<SourceListProps>);

pub fn source_list(id: impl Into<SharedString>) -> SourceList {
    let props = SourceListProps { sections: Vec::new(), selected: None, style: StyleRefinement::default() };
    SourceList(NativeElement::new::<SourceListKind>(Key::Id(id.into()), props).size_full())
}

impl SourceList {
    /// A titled group of rows.
    pub fn section(mut self, title: impl Into<SharedString>, items: impl IntoIterator<Item = SourceItem>) -> Self {
        let items = items.into_iter().collect();
        self.0.props_mut().sections.push(SourceSection { title: Some(title.into()), items });
        self
    }

    /// Rows with no group row above them.
    pub fn items(mut self, items: impl IntoIterator<Item = SourceItem>) -> Self {
        let items = items.into_iter().collect();
        self.0.props_mut().sections.push(SourceSection { title: None, items });
        self
    }

    pub fn selected(mut self, id: impl Into<SharedString>) -> Self {
        self.0.props_mut().selected = Some(id.into());
        self
    }

    pub fn on_select(self, f: impl Fn(&SourceSelected, &mut Window, &mut App) + 'static) -> Self {
        SourceList(self.0.on(f))
    }
}

impl Styled for SourceList {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl FluentBuilder for SourceList {}

impl IntoElement for SourceList {
    fn into_any_element(mut self) -> AnyElement {
        let style = self.0.style().clone();
        self.0.props_mut().style = style;
        self.0.into_any_element()
    }
}
