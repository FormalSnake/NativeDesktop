//! The drawn tab bar, view switcher and tab overview of the GPUI backends, in libadwaita's look:
//! `AdwTabBar` for document tabs (a group window's tabs, and `tab_strip` with `TabStyle::Strip`),
//! `AdwInlineViewSwitcher` (a toggle group) for the other strip styles, and `AdwTabOverview` for a
//! group window's overview. Colours and metrics come from the theme ([`TabLook`]).
//!
//! The builders are pure, as in [`crate::nav`]: a model, a theme and optional input handlers in,
//! an element out. The host and the strip's arm wire the handlers; goldens render them as they
//! are.

use std::cell::RefCell;
use std::rc::Rc;

use nd_components::{TabAddRequested, TabChanged, TabCloseRequested, TabStripProps, TabStyle};
use nd_core::{NodeId, Rect};
use nd_gpui::gpui::{
    self, AnyElement, Bounds, BoxShadow, ElementId, FontWeight, GlobalElementId, Hsla, InspectorElementId,
    InteractiveElement, IntoElement, LayoutId, MouseButton, MouseDownEvent, ParentElement, Pixels, Refineable,
    SharedString, StatefulInteractiveElement, Styled, WindowControlArea, anchored, deferred, div, point, px,
};
use nd_theme::Theme;

use crate::nav::{Marks, OnInput};
use crate::render::Build;

/// What the drawn tabs read from the theme. One constructor, as [`crate::forms`]' `FormLook`.
#[derive(Clone, Copy, Debug)]
pub struct TabLook {
    /// Height of a tab, a toggle track and the bar's end buttons.
    pub tab_height: Pixels,
    /// Space between the bar's edge and its tabs, and inside a toggle track.
    pub padding: Pixels,
    /// Horizontal space inside a tab and a toggle.
    pub inset: Pixels,
    pub radius: Pixels,
    /// Side of a tab's close button.
    pub button: Pixels,
    pub text_size: Pixels,
    /// The tab count inside the overview button.
    pub count_size: Pixels,
    pub background: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
    pub text: Hsla,
    pub disabled_text: Hsla,
    pub separator: Hsla,
    pub border: Hsla,
    /// A toggle group's track, and the selected toggle raised off it.
    pub track: Hsla,
    pub raised: Hsla,
    /// The selected card of the overview.
    pub focus: Hsla,
    pub card: Hsla,
    pub shadow: Hsla,
    /// Gaps of the overview grid and its margin.
    pub grid_gap: Pixels,
    pub margin: Pixels,
}

/// Width of an overview card, and the height of its thumbnail.
const CARD_WIDTH: f32 = 220.;
const THUMBNAIL_HEIGHT: f32 = 140.;

impl TabLook {
    /// `background` is what the bar sits on; the bar paints it, so the tabs read the same over any
    /// content.
    pub fn of(theme: &Theme) -> Self {
        let background = theme.surface.base;
        // libadwaita fills the selected tab with a tenth of the text colour and the hovered
        // one with a little less.
        let selected = theme.hover(background);
        Self {
            tab_height: theme.control.regular,
            padding: theme.spacing.xs,
            inset: theme.spacing.md,
            radius: theme.radius.md,
            button: theme.control.small,
            text_size: theme.type_scale.body,
            count_size: theme.type_scale.caption,
            background,
            hover: nd_theme::color::mix(background, selected, 0.6),
            selected,
            text: theme.text.primary,
            disabled_text: theme.disabled(theme.text.primary),
            separator: theme.separator,
            border: theme.border.subtle,
            track: selected,
            // The active toggle stands off the track: the raised surface on light schemes, where
            // it is lighter than the track, the raised hover surface on dark ones.
            raised: if theme.is_dark() { theme.surface.raised_hover } else { theme.surface.raised },
            focus: theme.focus_ring,
            card: theme.surface.raised,
            shadow: theme.scrim,
            grid_gap: theme.spacing.lg,
            margin: theme.spacing.xl,
        }
    }

    /// The bar's height: the tabs, the padding around them and the bottom hairline.
    pub fn bar_height(&self) -> f32 {
        self.tab_height.as_f32() + 2. * self.padding.as_f32() + 1.
    }
}

/// One tab as the drawn bar shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct BarTab {
    pub title: SharedString,
    pub closable: bool,
}

/// A bar to draw.
#[derive(Clone, Debug)]
pub struct Bar {
    pub tabs: Vec<BarTab>,
    pub selected: Option<usize>,
    /// Shows a `+` after the tabs.
    pub add: bool,
    /// Shows the overview button with the tab count.
    pub overview: bool,
    pub enabled: bool,
    /// Room the window's own controls take at the leading and trailing end, for a bar at the
    /// top of the window.
    pub insets: (f32, f32),
    /// Empty parts of the bar move the window.
    pub drag_region: bool,
    pub label: Option<SharedString>,
}

/// Input on a drawn bar or switcher.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BarInput {
    /// A press on a tab, in window coordinates: libadwaita selects on press, and a drag may
    /// follow.
    Press(usize, gpui::Point<Pixels>),
    Close(usize),
    Add,
    Overview,
}

/// Where the last paint put a bar's parts, in window coordinates, by tab index.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BarLayout {
    pub bar: Option<Rect>,
    pub tabs: Vec<Rect>,
    pub closes: Vec<Option<Rect>>,
    pub add: Option<Rect>,
    pub overview: Option<Rect>,
    /// The overview's cards, while it is open.
    pub cards: Vec<Rect>,
}

pub type LayoutSink = Rc<RefCell<BarLayout>>;

pub(crate) fn contains(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.origin.x && y >= r.origin.y && x < r.origin.x + r.size.width && y < r.origin.y + r.size.height
}

/// Records the bounds GPUI laid its child out at.
struct Measured {
    inner: AnyElement,
    sink: Box<dyn Fn(Rect)>,
}

impl IntoElement for Measured {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl gpui::Element for Measured {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> (LayoutId, ()) {
        (self.inner.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) {
        (self.sink)(Rect::new(
            bounds.origin.x.as_f32(),
            bounds.origin.y.as_f32(),
            bounds.size.width.as_f32(),
            bounds.size.height.as_f32(),
        ));
        self.inner.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) {
        self.inner.paint(window, cx);
    }
}

fn measured(
    inner: impl IntoElement,
    sink: Option<&LayoutSink>,
    f: impl Fn(&mut BarLayout, Rect) + 'static,
) -> AnyElement {
    match sink {
        Some(sink) => {
            let sink = sink.clone();
            Measured { inner: inner.into_any_element(), sink: Box::new(move |r| f(&mut sink.borrow_mut(), r)) }
                .into_any_element()
        }
        None => inner.into_any_element(),
    }
}

fn flat_button(id: ElementId, look: &TabLook, label: &str) -> gpui::Stateful<gpui::Div> {
    let hover = look.hover;
    div()
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(SharedString::from(label.to_owned()))
        .flex_none()
        .size(look.tab_height)
        .flex()
        .items_center()
        .justify_center()
        .rounded(look.radius)
        .hover(move |s| s.bg(hover))
}

/// The drawn `AdwTabBar`: tabs sharing the bar's width, the selected one filled, a close button
/// on the selected and the hovered tab, separators between unselected neighbours, and the
/// `+` and overview buttons at the end.
pub fn tab_bar_view(
    key: &str,
    bar: &Bar,
    theme: &Theme,
    on_input: Option<OnInput<BarInput>>,
    sink: Option<LayoutSink>,
    marks: &Marks,
) -> AnyElement {
    let look = TabLook::of(theme);
    let on_input = on_input.filter(|_| bar.enabled);
    let text = if bar.enabled { look.text } else { look.disabled_text };
    let regular = FontWeight::NORMAL;
    if let Some(sink) = &sink {
        *sink.borrow_mut() = BarLayout {
            tabs: vec![Rect::default(); bar.tabs.len()],
            closes: vec![None; bar.tabs.len()],
            ..BarLayout::default()
        };
    }
    marks.fill("tab bar", look.background, &[]);
    let under = [look.background];
    let mut row = div().flex().flex_row().items_center().flex_1().min_w_0().h(look.tab_height).occlude();
    for (i, tab) in bar.tabs.iter().enumerate() {
        let selected = bar.selected == Some(i);
        let next_selected = bar.selected == Some(i + 1);
        let group: SharedString = format!("{key}-tab-{i}").into();
        let mut tab_under = under.to_vec();
        if selected {
            marks.fill("selected tab", look.selected, &under);
            tab_under.push(look.selected);
        }
        marks.text(&tab.title, text, &tab_under, look.text_size, regular);
        let close = tab.closable.then(|| {
            marks.text("\u{00d7}", text, &tab_under, look.text_size, regular);
            let hover = theme.hover(if selected { look.selected } else { look.hover });
            let mut button = div()
                .id(ElementId::Name(format!("{key}-close-{i}").into()))
                .role(gpui::Role::Button)
                .aria_label(SharedString::from(format!("Close {}", tab.title)))
                .flex_none()
                .size(look.button)
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .hover(move |s| s.bg(hover))
                .child("\u{00d7}");
            if !selected {
                button = button.opacity(0.).group_hover(group.clone(), |s| s.opacity(1.));
            }
            if let Some(on_input) = on_input.clone() {
                button = button.on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    on_input(BarInput::Close(i), window, cx);
                });
            }
            measured(button, sink.as_ref(), move |l, r| {
                if let Some(slot) = l.closes.get_mut(i) {
                    *slot = Some(r);
                }
            })
        });
        let hover = look.hover;
        let mut element = div()
            .id(ElementId::Name(group.clone()))
            .group(group.clone())
            .role(gpui::Role::Tab)
            .aria_label(tab.title.clone())
            .aria_selected(selected)
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_row()
            .items_center()
            .px(look.padding)
            .gap(look.padding)
            .rounded(look.radius)
            .text_size(look.text_size)
            .text_color(text)
            // The close button's width on the leading side too, so the title centres in the tab.
            .child(div().flex_none().w(look.button))
            .child(div().flex_1().min_w_0().flex().justify_center().child(div().truncate().child(tab.title.clone())))
            .child(close.unwrap_or_else(|| div().flex_none().w(look.button).into_any_element()));
        element = if selected { element.bg(look.selected) } else { element.hover(move |s| s.bg(hover)) };
        if let Some(on_input) = on_input.clone() {
            let press = on_input.clone();
            let closes = sink.clone();
            element = element
                .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                    let on_close = closes.as_ref().is_some_and(|s| {
                        s.borrow()
                            .closes
                            .get(i)
                            .copied()
                            .flatten()
                            .is_some_and(|r| contains(&r, event.position.x.as_f32(), event.position.y.as_f32()))
                    });
                    if !on_close {
                        press(BarInput::Press(i, event.position), window, cx);
                    }
                })
                .when_some(tab.closable.then_some(()), |e, ()| {
                    e.on_mouse_up(MouseButton::Middle, move |_, window, cx| on_input(BarInput::Close(i), window, cx))
                });
        }
        row = row.child(measured(element, sink.as_ref(), move |l, r| {
            if let Some(slot) = l.tabs.get_mut(i) {
                *slot = r;
            }
        }));
        if i + 1 < bar.tabs.len() {
            let shown = !selected && !next_selected;
            if shown {
                marks.fill("tab separator", look.separator, &under);
            }
            row = row.child(
                div().flex_none().w(px(1.)).h(look.tab_height - look.inset).when(shown, |d| d.bg(look.separator)),
            );
        }
    }
    let mut bar_row = div()
        .id(ElementId::Name(format!("{key}-bar").into()))
        .role(gpui::Role::TabList)
        .aria_label(bar.label.clone().unwrap_or_else(|| "Tabs".into()))
        .w_full()
        .flex_none()
        .h(px(look.bar_height()))
        .flex()
        .flex_row()
        .items_center()
        .gap(look.padding)
        .pl(look.padding + px(bar.insets.0))
        .pr(look.padding + px(bar.insets.1))
        .bg(look.background)
        .border_b_1()
        .border_color(look.border)
        .text_color(text)
        .child(row);
    if bar.add {
        marks.text("+", text, &under, look.text_size, regular);
        let mut add = flat_button(ElementId::Name(format!("{key}-add").into()), &look, "New Tab")
            .text_size(look.text_size)
            .occlude()
            .child("+");
        if let Some(on_input) = on_input.clone() {
            add = add.on_click(move |_, window, cx| on_input(BarInput::Add, window, cx));
        }
        bar_row = bar_row.child(measured(add, sink.as_ref(), |l, r| l.add = Some(r)));
    }
    if bar.overview {
        let count = bar.tabs.len().to_string();
        marks.text(&count, text, &under, look.count_size, FontWeight::BOLD);
        let side = look.button - look.padding * 2.;
        let mut button =
            flat_button(ElementId::Name(format!("{key}-overview").into()), &look, "Show All Tabs").occlude().child(
                div()
                    .size(side)
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(theme.radius.sm)
                    .border_2()
                    .border_color(text)
                    .text_size(look.count_size)
                    .font_weight(FontWeight::BOLD)
                    .child(count),
            );
        if let Some(on_input) = on_input.clone() {
            button = button.on_click(move |_, window, cx| on_input(BarInput::Overview, window, cx));
        }
        bar_row = bar_row.child(measured(button, sink.as_ref(), |l, r| l.overview = Some(r)));
    }
    if bar.drag_region {
        bar_row = bar_row.window_control_area(WindowControlArea::Drag).on_mouse_down(
            MouseButton::Left,
            |event: &MouseDownEvent, window, _| {
                if event.click_count >= 2 {
                    window.zoom_window();
                } else {
                    crate::render::start_window_move(window);
                }
            },
        );
    }
    measured(bar_row, sink.as_ref(), |l, r| l.bar = Some(r))
}

/// The drawn `AdwInlineViewSwitcher`: a toggle group whose selected toggle is raised off the
/// track. The track's radius is the toggles' plus its padding, so the corners nest.
pub fn switcher_view(
    key: &str,
    bar: &Bar,
    theme: &Theme,
    on_input: Option<OnInput<BarInput>>,
    sink: Option<LayoutSink>,
    marks: &Marks,
) -> AnyElement {
    let look = TabLook::of(theme);
    let on_input = on_input.filter(|_| bar.enabled);
    let text = if bar.enabled { look.text } else { look.disabled_text };
    if let Some(sink) = &sink {
        *sink.borrow_mut() = BarLayout {
            tabs: vec![Rect::default(); bar.tabs.len()],
            closes: vec![None; bar.tabs.len()],
            ..BarLayout::default()
        };
    }
    let toggle_height = look.tab_height - look.padding * 2.;
    marks.fill("view switcher", look.track, &[]);
    let mut track = div()
        .id(ElementId::Name(format!("{key}-switcher").into()))
        .role(gpui::Role::TabList)
        .aria_label(bar.label.clone().unwrap_or_else(|| "Views".into()))
        .flex()
        .flex_row()
        .items_center()
        .h(look.tab_height)
        .p(look.padding)
        .gap(look.padding)
        .rounded(toggle_height / 2. + look.padding)
        .bg(look.track)
        .text_size(look.text_size)
        .text_color(text);
    for (i, tab) in bar.tabs.iter().enumerate() {
        let selected = bar.selected == Some(i);
        let mut under = vec![look.track];
        if selected {
            marks.fill("selected view", look.raised, &under);
            under.push(look.raised);
        }
        marks.text(&tab.title, text, &under, look.text_size, FontWeight::NORMAL);
        let hover = theme.hover(look.track);
        let mut toggle = div()
            .id(ElementId::Name(format!("{key}-view-{i}").into()))
            .role(gpui::Role::Tab)
            .aria_label(tab.title.clone())
            .aria_selected(selected)
            .h(toggle_height)
            .px(look.inset)
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .child(div().truncate().child(tab.title.clone()));
        toggle = if selected {
            toggle.bg(look.raised).border_1().border_color(look.border).shadow(vec![BoxShadow {
                color: look.shadow.opacity(0.3),
                offset: point(px(0.), px(1.)),
                blur_radius: look.padding,
                spread_radius: px(0.),
                inset: false,
            }])
        } else {
            toggle.hover(move |s| s.bg(hover))
        };
        if let Some(on_input) = on_input.clone() {
            toggle = toggle.on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                on_input(BarInput::Press(i, event.position), window, cx)
            });
        }
        track = track.child(measured(toggle, sink.as_ref(), move |l, r| {
            if let Some(slot) = l.tabs.get_mut(i) {
                *slot = r;
            }
        }));
    }
    measured(track, sink.as_ref(), |l, r| l.bar = Some(r))
}

/// Input on the overview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverviewInput {
    Pick(usize),
    /// A press on the overview outside every card.
    Dismiss,
}

/// The drawn `AdwTabOverview`: a card per tab over the window's background, the selected card
/// ringed. It fills the box it is placed in and takes every press there.
pub fn overview_view(
    key: &str,
    tabs: &[BarTab],
    selected: Option<usize>,
    theme: &Theme,
    on_input: Option<OnInput<OverviewInput>>,
    sink: Option<LayoutSink>,
    marks: &Marks,
) -> AnyElement {
    let look = TabLook::of(theme);
    if let Some(sink) = &sink {
        sink.borrow_mut().cards = vec![Rect::default(); tabs.len()];
    }
    marks.fill("tab overview", look.background, &[]);
    let mut grid = div()
        .id(ElementId::Name(format!("{key}-overview-grid").into()))
        .role(gpui::Role::Grid)
        .aria_label("Tabs")
        .flex()
        .flex_row()
        .flex_wrap()
        .content_start()
        .gap(look.grid_gap);
    for (i, tab) in tabs.iter().enumerate() {
        let picked = selected == Some(i);
        marks.fill("tab card", look.card, &[look.background]);
        marks.text(&tab.title, look.text, &[look.background], look.text_size, FontWeight::NORMAL);
        let thumbnail = div()
            .w(px(CARD_WIDTH))
            .h(px(THUMBNAIL_HEIGHT))
            .rounded(theme.radius.lg)
            .bg(look.card)
            .border_1()
            .border_color(look.border)
            .shadow(vec![BoxShadow {
                color: look.shadow.opacity(0.25),
                offset: point(px(0.), px(1.)),
                blur_radius: look.padding * 2.,
                spread_radius: px(0.),
                inset: false,
            }])
            .when(picked, |d| d.border_2().border_color(look.focus));
        let mut card = div()
            .id(ElementId::Name(format!("{key}-card-{i}").into()))
            .role(gpui::Role::GridCell)
            .aria_label(tab.title.clone())
            .aria_selected(picked)
            .w(px(CARD_WIDTH))
            .flex()
            .flex_col()
            .gap(look.padding * 2.)
            .child(thumbnail)
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .text_size(look.text_size)
                    .text_color(look.text)
                    .child(div().truncate().child(tab.title.clone())),
            );
        if let Some(on_input) = on_input.clone() {
            card = card.on_click(move |_, window, cx| {
                cx.stop_propagation();
                on_input(OverviewInput::Pick(i), window, cx)
            });
        }
        grid = grid.child(measured(card, sink.as_ref(), move |l, r| {
            if let Some(slot) = l.cards.get_mut(i) {
                *slot = r;
            }
        }));
    }
    let mut layer = div()
        .id(ElementId::Name(format!("{key}-overview").into()))
        .role(gpui::Role::Dialog)
        .aria_label("Tab Overview")
        .absolute()
        .inset_0()
        .p(look.margin)
        .bg(look.background)
        .occlude()
        .child(grid);
    if let Some(on_input) = on_input {
        layer = layer.on_click(move |_, window, cx| on_input(OverviewInput::Dismiss, window, cx));
    }
    layer.into_any_element()
}

/// What follows the pointer while a tab is torn out of its bar: the tab, lifted.
pub fn drag_preview(title: &SharedString, theme: &Theme, at: gpui::Point<Pixels>, width: f32) -> AnyElement {
    let look = TabLook::of(theme);
    let card = div()
        .w(px(width))
        .h(look.tab_height)
        .px(look.inset)
        .flex()
        .items_center()
        .justify_center()
        .rounded(look.radius)
        .bg(look.raised)
        .border_1()
        .border_color(look.border)
        .shadow(vec![BoxShadow {
            color: look.shadow.opacity(0.4),
            offset: point(px(0.), px(2.)),
            blur_radius: look.padding * 4.,
            spread_radius: px(0.),
            inset: false,
        }])
        .text_size(look.text_size)
        .text_color(look.text)
        .child(div().truncate().child(title.clone()));
    deferred(anchored().position(at).child(card)).with_priority(crate::overlay::PANEL_PRIORITY).into_any_element()
}

use gpui::prelude::FluentBuilder as _;

impl Build<'_> {
    /// `tab_strip` on GPUI: a view switcher, or the document tab bar for `TabStyle::Strip`.
    pub(crate) fn tab_strip(&self, id: NodeId, props: &TabStripProps) -> AnyElement {
        let theme = self.theme();
        let marks = Marks::default();
        let emit = self.backend.state.borrow().events.emitter(id);
        let wake = self.wake.clone();
        let selected = props.selected_index();
        let ids: Vec<SharedString> = props.tabs.iter().map(|t| t.id.clone()).collect();
        let on_input: OnInput<BarInput> = Rc::new(move |input, _window, cx| {
            match input {
                BarInput::Press(i, _) if Some(i) != selected => {
                    let Some(id) = ids.get(i) else { return };
                    emit.emit(TabChanged { id: id.clone() });
                }
                BarInput::Close(i) => {
                    let Some(id) = ids.get(i) else { return };
                    emit.emit(TabCloseRequested { id: id.clone() });
                }
                BarInput::Add => emit.emit(TabAddRequested),
                BarInput::Press(..) | BarInput::Overview => return,
            }
            wake(cx);
        });
        let strip = props.presentation == TabStyle::Strip;
        let bar = Bar {
            tabs: props.tabs.iter().map(|t| BarTab { title: t.title.clone(), closable: strip && t.closable }).collect(),
            selected,
            add: strip && props.add_button,
            overview: false,
            enabled: props.enabled,
            insets: (0., 0.),
            drag_region: false,
            label: props.label.clone(),
        };
        let sink = self.backend.tab_seam.borrow_mut().strips.entry(id).or_default().clone();
        let key = format!("nd-strip-{}", id.0);
        let element = if strip {
            tab_bar_view(&key, &bar, &theme, Some(on_input), Some(sink), &marks)
        } else {
            switcher_view(&key, &bar, &theme, Some(on_input), Some(sink), &marks)
        };
        self.record_marks(id, marks.take());
        let mut wrapper = div();
        wrapper.style().refine(&props.style);
        wrapper.flex().child(element).into_any_element()
    }
}
