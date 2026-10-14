//! `status_page(title)`: the empty state of a view that has nothing to show (no results, an
//! empty folder, an error), with an icon, a title, a description and the actions that get the
//! user out of it. The header is one widget kind the backend maps: SwiftUI's
//! `ContentUnavailableView` on macOS, which Apple restyles with the OS; a drawn libadwaita status
//! page on GPUI backends. The actions are ordinary buttons laid out under it.

use nd_gpui::gpui::Refineable;
use nd_ui::prelude::*;
use nd_ui::semantics::{Semantics, WidgetSemantics};
use nd_ui::{Key, Role, StyleRefinement};

use crate::icon::Icon;

#[derive(Clone, Debug, PartialEq)]
pub struct StatusPageProps {
    pub icon: Option<Icon>,
    pub title: SharedString,
    pub description: Option<SharedString>,
    /// Smaller icon and title, for a status page inside a pane or a popover.
    pub compact: bool,
    pub style: StyleRefinement,
}

/// Widget kind of the status page's header.
pub struct StatusPageKind;

impl WidgetSemantics for StatusPageProps {
    fn semantics(&self) -> Semantics {
        let mut s = Semantics::new(Role::Status, self.title.to_string());
        if let Some(description) = &self.description {
            s = s.value(description.to_string());
        }
        s
    }
}

pub struct StatusPage {
    header: NativeElement<StatusPageProps>,
    actions: Vec<AnyElement>,
    style: StyleRefinement,
}

/// An empty state titled `title`, centred in the room it is given.
pub fn status_page(title: impl Into<SharedString>) -> StatusPage {
    let title = title.into();
    let props = StatusPageProps {
        icon: None,
        title: title.clone(),
        description: None,
        compact: false,
        style: StyleRefinement::default(),
    };
    StatusPage {
        header: NativeElement::described::<StatusPageKind>(Key::Id(format!("status:{title}").into()), props),
        actions: Vec::new(),
        style: StyleRefinement::default(),
    }
}

impl StatusPage {
    pub fn icon(mut self, icon: Icon) -> Self {
        self.header.props_mut().icon = Some(icon);
        self
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.header.props_mut().description = Some(description.into());
        self
    }

    pub fn compact(mut self) -> Self {
        self.header.props_mut().compact = true;
        self
    }

    /// Keys the header, for automation and for two status pages side by side.
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        *self.header.key_mut() = Key::Id(id.into());
        self
    }
}

impl ParentElement for StatusPage {
    /// The actions: buttons shown under the description, the primary one first.
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.actions.extend(elements);
    }
}

impl Styled for StatusPage {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl FluentBuilder for StatusPage {}

impl IntoElement for StatusPage {
    fn into_any_element(mut self) -> AnyElement {
        let style = self.header.style().clone();
        self.header.props_mut().style = style;
        let actions = (!self.actions.is_empty()).then(|| h_flex().gap_2().justify_center().children(self.actions));
        let mut root =
            v_flex().size_full().items_center().justify_center().gap_4().child(self.header).children(actions);
        root.style().refine(&self.style);
        root.into_any_element()
    }
}
