//! In-window tabs: `tab_view(id)`, a tab strip over a stack of pages, and `tab_strip(id)` on its
//! own. Window tabs, where each tab is its own window, are `WindowOptions::tab_group`.
//!
//! The strip is one widget kind the backend maps: an `NSSegmentedControl` on macOS (what
//! `NSTabView` draws for its tabs), a drawn view switcher or tab bar on GPUI backends. The pages
//! share one grid cell, so the view measures as its biggest page and nothing below it moves
//! when the selection changes; pages not selected stay mounted and hidden, keeping their state.
//! Selection is controlled: the strip reports [`TabChanged`] and the app passes the id back.

use nd_gpui::gpui::Refineable;
use nd_ui::prelude::*;
use nd_ui::semantics::{Response, SemanticAction, Semantics, WidgetSemantics};
use nd_ui::{DispatchProps, Key, Role, StyleRefinement};

use crate::forms::guard;
use crate::icon::Icon;

/// How the strip draws its tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TabStyle {
    /// The platform's in-window tabs: a segmented control on macOS, a view switcher on GPUI.
    #[default]
    Automatic,
    /// Side by side in one segmented control.
    Segmented,
    /// Document tabs in a row, each closable, with an optional `+` at the end. macOS has no
    /// in-window document tab bar (its document tabs are window tabs), so it shows a segmented
    /// control there and a tab closes through the app's own command.
    Strip,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabSpec {
    pub id: SharedString,
    pub title: SharedString,
    pub icon: Option<Icon>,
    /// Shows a close button on the tab (strip style).
    pub closable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabStripProps {
    pub tabs: Vec<TabSpec>,
    /// The selected tab's id. `None` or an id no tab has selects the first tab.
    pub selected: Option<SharedString>,
    pub presentation: TabStyle,
    /// Shows a `+` after the tabs (strip style); a press reports [`TabAddRequested`].
    pub add_button: bool,
    /// Accessible name when nothing visible names the tabs.
    pub label: Option<SharedString>,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

impl TabStripProps {
    /// The tab shown: the selected one, else the first.
    pub fn selected_index(&self) -> Option<usize> {
        let found = self.selected.as_ref().and_then(|id| self.tabs.iter().position(|t| &t.id == id));
        found.or((!self.tabs.is_empty()).then_some(0))
    }

    /// The tab `value` names: its id, its title (exact, then ignoring case), or its index.
    pub fn tab_named(&self, value: &str) -> Option<&TabSpec> {
        let tabs = &self.tabs;
        tabs.iter()
            .find(|t| t.id.as_ref() == value)
            .or_else(|| tabs.iter().find(|t| t.title.as_ref() == value))
            .or_else(|| tabs.iter().find(|t| t.title.eq_ignore_ascii_case(value)))
            .or_else(|| value.trim().parse::<usize>().ok().and_then(|i| tabs.get(i)))
    }
}

/// The user picked a tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabChanged {
    pub id: SharedString,
}

/// The user pressed a tab's close button. The app removes the tab (or does not).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabCloseRequested {
    pub id: SharedString,
}

/// The user pressed the strip's `+`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabAddRequested;

/// Widget kind of the tab strip.
pub struct TabStripKind;

impl WidgetSemantics for TabStripProps {
    fn semantics(&self) -> Semantics {
        let name = self.label.clone().unwrap_or_default();
        let mut s = Semantics::new(Role::TabList, name.to_string()).enabled(self.enabled);
        if let Some(tab) = self.selected_index().and_then(|i| self.tabs.get(i)) {
            s = s.value(tab.title.to_string());
        }
        s
    }

    fn respond(&self, action: &SemanticAction) -> Option<Response> {
        let SemanticAction::SetValue(value) = action else { return None };
        if let Some(refused) = guard(self.enabled) {
            return Some(refused);
        }
        Some(match self.tab_named(value) {
            Some(tab) => Ok(Box::new(TabChanged { id: tab.id.clone() })),
            None => crate::forms::not_taken(&format!("no tab \"{value}\"")),
        })
    }
}

/// The tabs alone, for a strip that drives content the app lays out itself.
pub struct TabStrip(NativeElement<TabStripProps>);

pub fn tab_strip(id: impl Into<SharedString>) -> TabStrip {
    let props = TabStripProps {
        tabs: Vec::new(),
        selected: None,
        presentation: TabStyle::Automatic,
        add_button: false,
        label: None,
        enabled: true,
        tooltip: None,
        style: StyleRefinement::default(),
    };
    TabStrip(NativeElement::described::<TabStripKind>(Key::Id(id.into()), props))
}

impl TabStrip {
    pub fn tab(mut self, id: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        self.0.props_mut().tabs.push(TabSpec { id: id.into(), title: title.into(), icon: None, closable: false });
        self
    }

    pub fn tabs(mut self, tabs: impl IntoIterator<Item = TabSpec>) -> Self {
        self.0.props_mut().tabs.extend(tabs);
        self
    }

    pub fn selected(mut self, id: impl Into<SharedString>) -> Self {
        self.0.props_mut().selected = Some(id.into());
        self
    }

    pub fn presentation(mut self, presentation: TabStyle) -> Self {
        self.0.props_mut().presentation = presentation;
        self
    }

    pub fn add_button(mut self, shown: bool) -> Self {
        self.0.props_mut().add_button = shown;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.0.props_mut().label = Some(label.into());
        self
    }

    pub fn on_select(self, f: impl Fn(&TabChanged, &mut Window, &mut App) + 'static) -> Self {
        TabStrip(self.0.on(f))
    }

    pub fn on_close(self, f: impl Fn(&TabCloseRequested, &mut Window, &mut App) + 'static) -> Self {
        TabStrip(self.0.on(f))
    }

    pub fn on_add(self, f: impl Fn(&TabAddRequested, &mut Window, &mut App) + 'static) -> Self {
        TabStrip(self.0.on(f))
    }
}

crate::forms::leaf!(TabStrip, TabStripProps);

/// One page of a [`tab_view`]: its tab and its content.
pub struct TabPage {
    spec: TabSpec,
    children: Vec<AnyElement>,
}

pub fn tab_page(id: impl Into<SharedString>, title: impl Into<SharedString>) -> TabPage {
    TabPage { spec: TabSpec { id: id.into(), title: title.into(), icon: None, closable: false }, children: Vec::new() }
}

impl TabPage {
    pub fn icon(mut self, icon: Icon) -> Self {
        self.spec.icon = Some(icon);
        self
    }

    /// Shows a close button on the tab (strip style); a press reports [`TabCloseRequested`].
    pub fn closable(mut self) -> Self {
        self.spec.closable = true;
        self
    }
}

impl ParentElement for TabPage {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

/// A tab strip over a stack of pages.
pub struct TabView {
    id: SharedString,
    strip: TabStrip,
    pages: Vec<TabPage>,
    homogeneous: bool,
    style: StyleRefinement,
    dispatch: DispatchProps,
}

/// In-window tabs. `id` keys the strip, which is what automation finds (`setValue` with a tab's
/// title or id selects it); each page is keyed by its tab's id.
pub fn tab_view(id: impl Into<SharedString>) -> TabView {
    let id = id.into();
    TabView {
        strip: tab_strip(id.clone()),
        id,
        pages: Vec::new(),
        homogeneous: true,
        style: StyleRefinement::default(),
        dispatch: DispatchProps::default(),
    }
}

impl TabView {
    pub fn tab(mut self, page: TabPage) -> Self {
        self.pages.push(page);
        self
    }

    pub fn tabs(mut self, pages: impl IntoIterator<Item = TabPage>) -> Self {
        self.pages.extend(pages);
        self
    }

    pub fn selected(mut self, id: impl Into<SharedString>) -> Self {
        self.strip = self.strip.selected(id);
        self
    }

    pub fn presentation(mut self, presentation: TabStyle) -> Self {
        self.strip = self.strip.presentation(presentation);
        self
    }

    pub fn add_button(mut self, shown: bool) -> Self {
        self.strip = self.strip.add_button(shown);
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.strip = self.strip.label(label);
        self
    }

    /// Measures as the biggest page and keeps every page mounted (the default), or mounts only
    /// the selected page and takes its size.
    pub fn homogeneous(mut self, homogeneous: bool) -> Self {
        self.homogeneous = homogeneous;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.strip = self.strip.disabled(disabled);
        self
    }

    pub fn on_select(mut self, f: impl Fn(&TabChanged, &mut Window, &mut App) + 'static) -> Self {
        self.strip = self.strip.on_select(f);
        self
    }

    pub fn on_close(mut self, f: impl Fn(&TabCloseRequested, &mut Window, &mut App) + 'static) -> Self {
        self.strip = self.strip.on_close(f);
        self
    }

    pub fn on_add(mut self, f: impl Fn(&TabAddRequested, &mut Window, &mut App) + 'static) -> Self {
        self.strip = self.strip.on_add(f);
        self
    }
}

impl Styled for TabView {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl FluentBuilder for TabView {}

impl nd_ui::InteractiveElement for TabView {
    fn dispatch_props(&mut self) -> &mut DispatchProps {
        &mut self.dispatch
    }
}

impl IntoElement for TabView {
    fn into_any_element(self) -> AnyElement {
        let specs: Vec<TabSpec> = self.pages.iter().map(|p| p.spec.clone()).collect();
        let strip = self.strip.tabs(specs);
        let shown = strip.props().selected_index();
        let homogeneous = self.homogeneous;
        let pages = self.pages.into_iter().enumerate().filter_map(|(i, page)| {
            let selected = Some(i) == shown;
            (homogeneous || selected).then(|| {
                div()
                    .id(page.spec.id.clone())
                    .role(Role::TabPanel, page.spec.title.clone())
                    .row_start(1)
                    .col_start(1)
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .when(!selected, |d| d.invisible())
                    .children(page.children)
            })
        });
        let stack = div().flex_1().min_h_0().grid().grid_cols(1).grid_rows(1).children(pages);
        // Document tabs run the width of the view; switcher tabs sit centered over it.
        let strip = match strip.props().presentation {
            TabStyle::Strip => strip.self_stretch(),
            TabStyle::Automatic | TabStyle::Segmented => strip.self_center(),
        };
        let mut root = v_flex().id(format!("{}.view", self.id)).child(strip).child(stack);
        root.style().refine(&self.style);
        *nd_ui::InteractiveElement::dispatch_props(&mut root) = self.dispatch;
        root.into_any_element()
    }
}
