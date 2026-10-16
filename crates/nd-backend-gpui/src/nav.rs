//! The drawn command palette, source tree, toasts and banner: what the GPUI backends show for
//! the widgets AppKit maps to its own classes, in the Linux look (libadwaita metrics, the
//! matugen-ready palette). Behaviour is the shared model's ([`PaletteModel`], [`ToastQueue`],
//! [`SourceIndex`]); this module draws it and turns input into it.
//!
//! The view builders are pure: a model snapshot, a [`Theme`] and optional input handlers in, an
//! element out. The backend wires the handlers; goldens render the builders as they are.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::forms::glyph::{Glyph, glyph};
use nd_components::{
    BANNER, BannerProps, CommandPaletteProps, FieldSelection, PALETTE, PaletteKey, PaletteModel, SourceIndex,
    SourceTreeProps, TOAST, Toast, ToastOverlayProps, ToastQueue,
};
use nd_core::{NodeId, WindowId};
use nd_gpui::gpui::{
    self, Animation, AnimationExt, AnyElement, BoxShadow, ElementId, FocusHandle, FontWeight, Hsla, InteractiveElement,
    IntoElement, Keystroke, MouseButton, ParentElement, Pixels, SharedString, StatefulInteractiveElement, Styled,
    anchored, deferred, div, point, px, relative,
};
use nd_theme::color::with_alpha;
use nd_theme::{Metrics, Palette, Preset, Scheme, Theme};

/// The baked theme for `scheme`, for rendering the builders outside an app. In an app they get
/// the window's resolved theme.
pub fn theme_for(scheme: Scheme) -> Theme {
    let preset = if cfg!(target_os = "windows") { Preset::Fluent } else { Preset::Adwaita };
    Theme::derive(preset, scheme, Palette::baked(scheme), Metrics::preset(preset), 1.)
}

/// One colour a drawn widget paints at rest, for the backend's paint record (the appearance
/// audit reads it): `under` are the widget's own fills beneath it, innermost last.
#[derive(Clone, Debug)]
pub struct Mark {
    pub label: String,
    pub color: Hsla,
    pub under: Vec<Hsla>,
    /// Text size and CSS weight; `None` for a fill.
    pub text: Option<(f32, f32)>,
}

/// Where the builders leave their [`Mark`]s.
#[derive(Default)]
pub struct Marks(RefCell<Vec<Mark>>);

impl Marks {
    pub(crate) fn fill(&self, label: &str, color: Hsla, under: &[Hsla]) {
        self.0.borrow_mut().push(Mark { label: label.into(), color, under: under.to_vec(), text: None });
    }

    pub(crate) fn text(&self, label: &str, color: Hsla, under: &[Hsla], size: Pixels, weight: FontWeight) {
        let text = Some((size.as_f32(), weight.0));
        self.0.borrow_mut().push(Mark { label: label.into(), color, under: under.to_vec(), text });
    }

    pub fn take(&self) -> Vec<Mark> {
        self.0.take()
    }
}

/// Input the drawn palette turns into model calls.
#[derive(Clone, Debug, PartialEq)]
pub enum PaletteInput {
    Key(PaletteKey),
    Insert(String),
    Backspace,
    Click(usize),
    /// A click outside the card.
    Cancel,
}

pub type OnInput<I> = Rc<dyn Fn(I, &mut gpui::Window, &mut gpui::App)>;

/// A banner's button handler.
pub type OnAction = Rc<dyn Fn(&mut gpui::App)>;

/// What a keystroke means to the palette's field. `None` leaves it to the window.
pub fn palette_input(keystroke: &Keystroke) -> Option<PaletteInput> {
    let m = &keystroke.modifiers;
    let key = |k| Some(PaletteInput::Key(k));
    match keystroke.key.as_str() {
        "up" => key(PaletteKey::Up),
        "down" => key(PaletteKey::Down),
        "home" => key(PaletteKey::Home),
        "end" => key(PaletteKey::End),
        "tab" if m.shift => key(PaletteKey::BackTab),
        "tab" => key(PaletteKey::Tab),
        "right" => key(PaletteKey::Right),
        "enter" if m.platform || m.control => key(PaletteKey::SecondaryReturn),
        "enter" => key(PaletteKey::Return),
        "escape" => key(PaletteKey::Escape),
        "backspace" => Some(PaletteInput::Backspace),
        _ if m.control || m.platform => None,
        // A keystroke synthesized from its key name carries no typed character: a one-character
        // key name is that character.
        key => keystroke
            .key_char
            .clone()
            .or_else(|| (key.chars().count() == 1).then(|| if m.shift { key.to_uppercase() } else { key.to_owned() }))
            .filter(|c| !c.is_empty())
            .map(PaletteInput::Insert),
    }
}

/// The glyph an icon slot draws, or `None` so the slot is not laid out at all.
pub(crate) fn icon_glyph(icon: Option<nd_components::Icon>) -> Option<Glyph> {
    icon.filter(|i| i.shape().is_some()).map(Glyph::of)
}

/// The presented palette over a window of `viewport`: the scrim, the card, the field and the
/// rows. Nothing when the model is not presented.
pub fn palette_overlay(
    model: &PaletteModel,
    placeholder: Option<&str>,
    theme: &Theme,
    viewport: gpui::Size<gpui::Pixels>,
    focus: Option<&FocusHandle>,
    on_input: Option<OnInput<PaletteInput>>,
    marks: &Marks,
) -> AnyElement {
    if !model.is_presented() {
        return div().into_any_element();
    }
    let m = PALETTE;
    let (w, h) = (viewport.width.as_f32(), viewport.height.as_f32());
    let card_w = m.width.min(w - 2. * m.margin).max(0.);
    let top = (h * m.top_fraction).max(m.margin);
    let line = m.field_font_size * 1.25;
    let rows = model.rows();
    let scrim = theme.scrim;
    marks.fill("palette scrim", scrim, &[]);
    marks.fill("palette card", theme.surface.overlay, &[scrim]);
    let card_under = [scrim, theme.surface.overlay];
    let field_size = px(m.field_font_size);
    let regular = FontWeight::NORMAL;

    let mut field = div()
        .id("nd-palette-field")
        .flex()
        .flex_row()
        .items_center()
        .min_w_0()
        .flex_1()
        .h(px(line))
        .text_size(px(m.field_font_size));
    let text = model.field_text();
    let selected = [scrim, theme.surface.overlay, theme.selection];
    if text.is_empty() {
        // Secondary, not tertiary: a placeholder is read text and needs 4.5:1.
        let placeholder = placeholder.unwrap_or("");
        marks.text(placeholder, theme.text.secondary, &card_under, field_size, regular);
        field =
            field.child(caret(theme, line)).child(div().text_color(theme.text.secondary).child(placeholder.to_owned()));
    } else {
        match model.selection() {
            FieldSelection::All => {
                marks.fill("field selection", theme.selection, &card_under);
                marks.text(&text, theme.text.primary, &selected, field_size, regular);
                field = field.child(div().bg(theme.selection).text_color(theme.text.primary).child(text));
            }
            FieldSelection::Caret => {
                marks.text(model.typed(), theme.text.primary, &card_under, field_size, regular);
                field = field
                    .child(div().text_color(theme.text.primary).child(model.typed().to_owned()))
                    .child(caret(theme, line));
            }
            FieldSelection::Completion => {
                marks.text(model.typed(), theme.text.primary, &card_under, field_size, regular);
                marks.fill("field selection", theme.selection, &card_under);
                marks.text(model.completion(), theme.text.primary, &selected, field_size, regular);
                field = field.child(div().text_color(theme.text.primary).child(model.typed().to_owned())).child(
                    div().bg(theme.selection).text_color(theme.text.primary).child(model.completion().to_owned()),
                );
            }
        }
    }

    let mut list = div()
        .id("nd-palette-results")
        .role(gpui::Role::ListBox)
        .aria_label("Results")
        .flex()
        .flex_col()
        .max_h(px(m.max_list_height))
        .overflow_y_scroll()
        .px(theme.spacing.sm)
        .py(px(m.list_gap));
    for (i, item) in rows.iter().enumerate() {
        let highlighted = model.highlight() == Some(i);
        let (bg, fg, secondary) = if highlighted {
            (Some(theme.accent_container.bg), theme.accent_container.fg, theme.accent_container.fg.opacity(0.8))
        } else {
            (None, theme.text.primary, theme.text.secondary)
        };
        let mut under = card_under.to_vec();
        if let Some(bg) = bg {
            marks.fill("palette row", bg, &under);
            under.push(bg);
        }
        let body = theme.type_scale.body;
        marks.text(&item.title, fg, &under, body, regular);
        for quiet in item.subtitle.iter().chain(item.hint.iter()) {
            marks.text(quiet, secondary, &under, body, regular);
        }
        let mut row = div()
            .id(ElementId::NamedInteger("nd-palette-row".into(), i as u64))
            .role(gpui::Role::ListBoxOption)
            .aria_label(item.accessible_label())
            .aria_selected(highlighted)
            .flex()
            .flex_row()
            .items_center()
            .h(px(m.row_height))
            .px(px(m.cell_inset))
            .gap(px(m.cell_gap))
            .rounded(theme.radius.md)
            .text_size(theme.type_scale.body)
            .text_color(fg)
            // A row without an icon (or whose icon draws nothing) has no slot at all.
            .children(icon_glyph(item.icon).map(|g| glyph(g, px(m.icon_side), fg, px(1.5))))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_w_0()
                    .gap(px(m.subtitle_gap))
                    .child(div().flex_shrink_0().max_w(relative(m.title_share)).truncate().child(item.title.clone()))
                    .children(item.subtitle.clone().map(|s| div().min_w_0().truncate().text_color(secondary).child(s))),
            )
            .children(
                item.hint
                    .clone()
                    .map(|hint| div().flex_shrink_0().pl(px(m.hint_gap)).text_color(secondary).child(hint)),
            );
        if let Some(bg) = bg {
            row = row.bg(bg);
        } else {
            let hover = theme.hover(theme.surface.overlay);
            row = row.hover(move |s| s.bg(hover));
        }
        if let Some(on_input) = on_input.clone() {
            row = row.on_click(move |_, window, cx| on_input(PaletteInput::Click(i), window, cx));
        }
        list = list.child(row);
    }

    let mut card = div()
        .id("nd-palette-card")
        .role(gpui::Role::Dialog)
        .aria_label(placeholder.unwrap_or("Command Palette").to_owned())
        .absolute()
        .left(px(((w - card_w) / 2.).round()))
        .top(px(top.round()))
        .w(px(card_w))
        .flex()
        .flex_col()
        .bg(theme.surface.overlay)
        .border_1()
        .border_color(theme.border.subtle)
        .rounded(px(m.radius))
        .shadow(vec![BoxShadow {
            color: gpui::black().opacity(m.shadow_opacity),
            offset: point(px(0.), px(0.)),
            blur_radius: px(m.shadow_radius),
            spread_radius: px(0.),
            inset: false,
        }])
        .occlude()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .pt(px(m.field_top))
                .pl(px(m.search_icon_inset))
                .pr(px(m.field_trailing))
                .gap(px(m.search_icon_gap))
                .child(glyph(Glyph::Search, px(m.search_icon), theme.text.secondary, px(1.75)))
                .child(field.role(gpui::Role::SearchInput).aria_value(model.field_text())),
        );
    if !rows.is_empty() {
        marks.fill("palette separator", theme.separator, &card_under);
    }
    if rows.is_empty() {
        card = card.child(div().h(px(m.separator_gap + 1. + m.list_gap)));
    } else {
        card = card.child(div().mt(px(m.separator_gap)).h(px(1.)).bg(theme.separator)).child(list.mt(px(m.list_gap)));
    }
    if let Some(focus) = focus {
        card = card.track_focus(focus);
    }
    if let Some(on_input) = on_input.clone() {
        card = card.on_key_down(move |event, window, cx| {
            if let Some(input) = palette_input(&event.keystroke) {
                cx.stop_propagation();
                on_input(input, window, cx);
            }
        });
    }

    let mut scrim = div()
        .id("nd-palette-backdrop")
        .size_full()
        .w(viewport.width)
        .h(viewport.height)
        .bg(scrim)
        .occlude()
        .child(card);
    if let Some(on_input) = on_input {
        scrim = scrim.on_mouse_down(MouseButton::Left, move |_, window, cx| on_input(PaletteInput::Cancel, window, cx));
    }
    deferred(anchored().position(point(px(0.), px(0.))).child(scrim))
        .with_priority(crate::overlay::PANEL_PRIORITY)
        .into_any_element()
}

fn caret(theme: &Theme, line: f32) -> gpui::Div {
    div().w(px(1.)).h(px(line * 0.8)).bg(theme.accent)
}

/// Input on a drawn source tree.
#[derive(Clone, Debug, PartialEq)]
pub enum TreeInput {
    Select(SharedString),
    Activate(SharedString),
    /// The disclosure: `true` asks to expand.
    Toggle(SharedString, bool),
    Action(SharedString, SharedString),
    Hover(Option<SharedString>),
}

/// The drawn source tree: section headers, rows indented by depth with a disclosure, the
/// badge and the trailing actions, and the empty state when there are no rows.
pub fn source_tree_view(
    id: NodeId,
    props: &SourceTreeProps,
    hovered: Option<&SharedString>,
    theme: &Theme,
    compact: bool,
    on_input: Option<OnInput<TreeInput>>,
    marks: &Marks,
) -> AnyElement {
    let (body, footnote) = (theme.type_scale.body, theme.type_scale.footnote);
    // WinUI's NavigationView: an icon per item, a subtle fill with an accent pill on the selected
    // one, and an icon-only rail when the pane is compact.
    let fluent = theme.preset == Preset::Fluent;
    let subtle = with_alpha(
        if theme.is_dark() { gpui::white() } else { gpui::black() },
        if theme.is_dark() { 0.06 } else { 0.04 },
    );
    let (regular, semibold) = (FontWeight::NORMAL, FontWeight::SEMIBOLD);
    let mut list = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::Tree)
        .flex()
        .flex_col()
        .size_full()
        .overflow_y_scroll()
        .p(theme.spacing.sm)
        .gap(px(1.))
        .text_size(theme.type_scale.body);
    if props.nodes.is_empty() {
        if let Some(empty) = &props.empty {
            marks.text(&empty.title, theme.text.secondary, &[], body, semibold);
            if let Some(description) = &empty.description {
                marks.text(description, theme.text.secondary, &[], footnote, regular);
            }
            list = list.justify_center().items_center().gap(theme.spacing.xs).child(
                div().text_color(theme.text.secondary).font_weight(FontWeight::SEMIBOLD).child(empty.title.clone()),
            );
            if let Some(description) = &empty.description {
                list = list.child(
                    div()
                        .text_size(theme.type_scale.footnote)
                        .text_color(theme.text.secondary)
                        .child(description.clone()),
                );
            }
        }
        return list.into_any_element();
    }
    let index = SourceIndex::new(&props.nodes);
    let actions: HashMap<&SharedString, _> = props.actions.iter().map(|a| (&a.id, a)).collect();
    let hover_only = props.action_visibility == nd_components::ActionVisibility::Hover;
    let indent = theme.spacing.lg.as_f32();
    for (i, depth) in index.visible(&props.nodes) {
        let node = &props.nodes[i];
        if node.section && compact {
            continue;
        }
        if node.section {
            marks.text(&node.title, theme.text.secondary, &[], footnote, semibold);
            list = list.child(
                div()
                    .pt(theme.spacing.md)
                    .pb(theme.spacing.sm)
                    .px(theme.spacing.sm)
                    .text_size(theme.type_scale.footnote)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text.secondary)
                    .child(node.title.clone()),
            );
            continue;
        }
        let selected = props.selected.as_ref() == Some(&node.id);
        let expandable = index.expandable(&props.nodes, i);
        let (fg, secondary) = if selected && !fluent {
            (theme.accent_container.fg, theme.accent_container.fg.opacity(0.8))
        } else {
            (theme.text.primary, theme.text.secondary)
        };
        let selected_fill = if fluent { subtle } else { theme.accent_container.bg };
        let mut under = Vec::new();
        if selected {
            marks.fill("selected row", selected_fill, &[]);
            under.push(selected_fill);
        }
        let show_actions = !hover_only || hovered == Some(&node.id);
        if expandable {
            marks.text(if node.expanded { "\u{25be}" } else { "\u{25b8}" }, secondary, &under, body, regular);
        }
        marks.text(&node.title, fg, &under, body, regular);
        for quiet in node.caption.iter().chain(node.badge.iter()) {
            marks.text(quiet, secondary, &under, footnote, regular);
        }
        for action in node.actions.iter().filter_map(|a| actions.get(a)).filter(|_| show_actions) {
            let color = if action.destructive { theme.danger } else { secondary };
            let label = action.label.clone().unwrap_or_else(|| action.accessible_label());
            marks.text(&label, color, &under, footnote, regular);
        }
        let key = node.id.clone();
        let mut disclosure = div()
            .id(ElementId::Name(format!("{}-disclosure", node.id).into()))
            .flex_shrink_0()
            .w(theme.spacing.lg)
            .text_color(secondary)
            .child(if expandable { if node.expanded { "\u{25be}" } else { "\u{25b8}" } } else { "" });
        if expandable && let Some(on_input) = on_input.clone() {
            let (key, expand) = (key.clone(), !node.expanded);
            disclosure = disclosure.on_click(move |_, window, cx| {
                cx.stop_propagation();
                on_input(TreeInput::Toggle(key.clone(), expand), window, cx);
            });
        }
        let icon = node
            .icon
            .filter(|_| fluent)
            .and_then(|icon| icon.segoe_fluent())
            .map(|code| crate::fluent_icons::glyph(code, px(16.), fg));
        let pill = (fluent && selected).then(|| {
            div()
                .absolute()
                .left_0()
                .top(px((theme.control.regular.as_f32() - 16.) / 2.))
                .w(px(3.))
                .h(px(16.))
                .rounded(px(1.5))
                .bg(theme.accent)
        });
        let mut row = div()
            .id(ElementId::Name(node.id.clone()))
            .role(gpui::Role::TreeItem)
            .aria_label(node.title.clone())
            .aria_selected(selected)
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .min_h(theme.control.regular)
            .rounded(theme.radius.md)
            .text_color(fg)
            .children(pill);
        if compact {
            row = row.justify_center().child(icon.unwrap_or_else(|| {
                crate::fluent_icons::glyph(
                    nd_components::Icon::FileText.segoe_fluent().unwrap_or_default(),
                    px(16.),
                    fg,
                )
            }));
        } else {
            row = row
                .pl(px(depth as f32 * indent))
                .pr(theme.spacing.sm)
                .child(disclosure)
                .children(match icon {
                    Some(icon) => Some(icon.mr(theme.spacing.md).into_any_element()),
                    None => icon_glyph(node.icon).map(|g| {
                        div()
                            .flex_shrink_0()
                            .pr(theme.spacing.sm)
                            .child(glyph(g, px(16.), secondary, px(1.5)))
                            .into_any_element()
                    }),
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(div().truncate().child(node.title.clone()))
                        .children(node.caption.clone().map(|c| {
                            div().truncate().text_size(theme.type_scale.footnote).text_color(secondary).child(c)
                        })),
                )
                .children(node.badge.clone().map(|b| {
                    div()
                        .flex_shrink_0()
                        .pl(theme.spacing.sm)
                        .text_size(theme.type_scale.footnote)
                        .text_color(secondary)
                        .child(b)
                }));
        }
        if expandable {
            row = row.aria_expanded(node.expanded);
        }
        for action_id in node.actions.iter().filter(|_| !compact) {
            let Some(action) = actions.get(action_id) else { continue };
            let label = action.label.clone().unwrap_or_else(|| action.accessible_label());
            let color = if action.destructive { theme.danger } else { secondary };
            let mut button = div()
                .id(ElementId::Name(format!("{}-{}", node.id, action.id).into()))
                .role(gpui::Role::Button)
                .aria_label(action.accessible_label())
                .flex_shrink_0()
                .ml(theme.spacing.xs)
                .px(theme.spacing.xs)
                .rounded(theme.radius.sm)
                .text_size(theme.type_scale.footnote)
                .text_color(color)
                .opacity(if show_actions { 1. } else { 0. })
                .child(label);
            if show_actions && let Some(on_input) = on_input.clone() {
                let (key, action) = (key.clone(), action.id.clone());
                button = button.on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    on_input(TreeInput::Action(key.clone(), action.clone()), window, cx);
                });
            }
            row = row.child(button);
        }
        if selected {
            row = row.bg(selected_fill);
        } else {
            let hover = if fluent { subtle } else { theme.hover(theme.surface.sidebar) };
            row = row.hover(move |s| s.bg(hover));
        }
        if let Some(on_input) = on_input.clone() {
            let selectable = node.selectable;
            let click_key = key.clone();
            let on_click = on_input.clone();
            row = row.on_click(move |event, window, cx| {
                if event.click_count() >= 2 {
                    on_click(TreeInput::Activate(click_key.clone()), window, cx);
                } else if selectable {
                    on_click(TreeInput::Select(click_key.clone()), window, cx);
                }
            });
            if hover_only {
                row = row.on_hover(move |inside, window, cx| {
                    on_input(TreeInput::Hover(inside.then(|| key.clone())), window, cx);
                });
            }
        }
        list = list.child(row);
    }
    list.into_any_element()
}

/// Input on a drawn toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastInput {
    Body,
    Action,
}

/// The toast on screen, bottom-centre over a window of `viewport`, sliding up as it fades in.
pub fn toast_view(
    toast: Option<&Toast>,
    theme: &Theme,
    viewport: gpui::Size<gpui::Pixels>,
    animate: bool,
    on_input: Option<OnInput<ToastInput>>,
    marks: &Marks,
) -> AnyElement {
    let Some(toast) = toast else { return div().into_any_element() };
    let t = TOAST;
    let under = [theme.surface.overlay];
    marks.fill("toast", theme.surface.overlay, &[]);
    marks.text(&toast.title, theme.text.primary, &under, theme.type_scale.body, FontWeight::NORMAL);
    if let Some(label) = &toast.action {
        marks.text(label, theme.text.primary, &under, theme.type_scale.body, FontWeight::SEMIBOLD);
    }
    let mut body = div()
        .id(ElementId::Name(format!("nd-toast-{}", toast.id).into()))
        .role(gpui::Role::Status)
        .aria_label(toast.title.clone())
        .flex()
        .flex_row()
        .items_center()
        .gap(px(t.spacing))
        .max_w(px(t.max_width.min(viewport.width.as_f32() - 2. * t.padding_x)))
        .pl(px(t.padding_x))
        .pr(px(if toast.action.is_some() { t.padding_action } else { t.padding_x }))
        .py(px(t.padding_y))
        .rounded(px(t.radius))
        .bg(theme.surface.overlay)
        .border_1()
        .border_color(theme.border.subtle)
        .shadow_lg()
        .text_size(theme.type_scale.body)
        .text_color(theme.text.primary)
        .occlude()
        .child(div().min_w_0().truncate().child(toast.title.clone()));
    if let Some(label) = &toast.action {
        let mut button = div()
            .id("nd-toast-action")
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .flex_shrink_0()
            .px(theme.spacing.sm)
            .rounded(theme.radius.sm)
            // libadwaita's toast button is a flat button in the window's text colour; the
            // accent is under 4.5:1 on the dark overlay.
            .text_color(theme.text.primary)
            .font_weight(FontWeight::SEMIBOLD)
            .child(label.clone());
        if let Some(on_input) = on_input.clone() {
            button = button.on_click(move |_, window, cx| {
                cx.stop_propagation();
                on_input(ToastInput::Action, window, cx);
            });
        }
        body = body.child(button);
    }
    if let Some(on_input) = on_input {
        body = body.on_click(move |_, window, cx| on_input(ToastInput::Body, window, cx));
    }
    let rise = t.rise;
    let body: AnyElement = if animate {
        body.with_animation(
            ElementId::Name(format!("nd-toast-enter-{}", toast.id).into()),
            Animation::new(t.enter).with_easing(gpui::ease_out_quint()),
            move |el, delta| el.opacity(delta).mt(px(rise * (1. - delta))),
        )
        .into_any_element()
    } else {
        body.into_any_element()
    };
    let layer = div()
        .w(viewport.width)
        .h(viewport.height)
        .flex()
        .flex_col()
        .justify_end()
        .items_center()
        .pb(px(t.bottom))
        .child(body);
    deferred(anchored().position(point(px(0.), px(0.))).child(layer))
        .with_priority(crate::overlay::TOAST_PRIORITY)
        .into_any_element()
}

/// The drawn banner: the title and an optional button on the accent container.
pub fn banner_view(
    id: NodeId,
    props: &BannerProps,
    theme: &Theme,
    on_action: Option<OnAction>,
    marks: &Marks,
) -> AnyElement {
    if !props.revealed {
        return div().into_any_element();
    }
    let (bg, fg) = (theme.accent_container.bg, theme.accent_container.fg);
    let button = theme.pressed(bg);
    marks.fill("banner", bg, &[]);
    marks.text(&props.title, fg, &[bg], theme.type_scale.body, FontWeight::NORMAL);
    if let Some(label) = &props.action {
        marks.fill("banner button", button, &[bg]);
        marks.text(label, fg, &[bg, button], theme.type_scale.body, FontWeight::SEMIBOLD);
    }
    let mut bar = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::Status)
        .aria_label(props.title.clone())
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .gap(px(BANNER.spacing))
        .px(px(BANNER.padding_x))
        .py(px(BANNER.padding_y))
        .bg(theme.accent_container.bg)
        .text_color(theme.accent_container.fg)
        .text_size(theme.type_scale.body)
        .child(div().flex_1().min_w_0().truncate().child(props.title.clone()));
    if let Some(label) = &props.action {
        let mut button = div()
            .id("nd-banner-action")
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .flex()
            .flex_shrink_0()
            .items_center()
            .h(theme.control.regular)
            .px(theme.spacing.md)
            .rounded(theme.radius.md)
            .bg(button)
            .font_weight(FontWeight::SEMIBOLD)
            .child(label.clone());
        if let Some(on_action) = on_action {
            button = button.on_click(move |_, _, cx| on_action(cx));
        }
        bar = bar.child(button);
    }
    // WinUI's InfoBar is a card inset in the content, where libadwaita's banner spans it edge to
    // edge; inset, it also keeps clear of the rounded corner of a Mica window's content layer.
    if theme.preset == nd_theme::Preset::Fluent {
        return div().w_full().p(theme.spacing.md).child(bar.rounded(theme.radius.md)).into_any_element();
    }
    bar.into_any_element()
}

/// Per-node state of the drawn widgets, kept beside the retained tree and dropped on release.
pub(crate) enum Drawn {
    Palette(PaletteDrawn),
    Toasts(ToastsDrawn),
    Tree(TreeDrawn),
}

pub(crate) struct PaletteDrawn {
    pub model: PaletteModel,
    pub focus: FocusHandle,
    /// Where focus was before the palette took it.
    pub previous: Option<FocusHandle>,
    pub applied: Option<CommandPaletteProps>,
}

#[derive(Default)]
pub(crate) struct ToastsDrawn {
    pub queue: ToastQueue,
    pub applied: Option<ToastOverlayProps>,
}

#[derive(Default)]
pub(crate) struct TreeDrawn {
    pub hovered: Option<SharedString>,
}

pub(crate) type DrawnMap = Rc<RefCell<HashMap<NodeId, Rc<RefCell<Drawn>>>>>;

use nd_components::{
    BannerActionClicked, BannerKind, CommandPaletteKind, SourceActionClicked, SourceActivated, SourceCollapsed,
    SourceExpanded, SourceSelected, SourceTreeKind, ToastChange, ToastOverlayKind, emit_palette_event,
};
use nd_ui::backend::{NodeKind, WidgetKind};
use nd_ui::events::Emit;

use crate::backend::{GpuiBackend, Node};
use crate::render::Build;

fn is<W: 'static>(kind: WidgetKind) -> bool {
    kind.type_id == std::any::TypeId::of::<W>()
}

fn kind_is<W: 'static>(node: &Node) -> bool {
    matches!(node.kind, NodeKind::Widget(w) if is::<W>(w))
}

/// Reports toast changes to the app; true when something was reported.
fn report_toasts(emit: &Emit, changes: Vec<ToastChange>) -> bool {
    let mut reported = false;
    for change in changes {
        match change {
            ToastChange::Dismissed(e) => {
                emit.emit(e);
                reported = true;
            }
            ToastChange::Action(e) => {
                emit.emit(e);
                reported = true;
            }
            ToastChange::Show(_) | ToastChange::Refresh(_) | ToastChange::Hide => {}
        }
    }
    reported
}

impl GpuiBackend {
    /// A drawn palette's text and whether it is open; the toast on screen's title.
    pub(crate) fn probe_nav(&self, node: NodeId, probe: &mut nd_ui::semantics::Probe) {
        let Some(drawn) = self.drawn_of(node) else { return };
        let Ok(drawn) = drawn.try_borrow() else { return };
        match &*drawn {
            Drawn::Palette(p) => {
                probe.value = Some(p.model.field_text());
                probe.expanded = Some(p.model.is_presented());
            }
            Drawn::Toasts(t) => probe.name = t.queue.current().map(|t| t.title.to_string()),
            Drawn::Tree(_) => {}
        }
    }

    /// A presented palette with focus in `window` takes every key without the secondary modifier
    /// (Tab, the arrows, Return, Esc and typing) and secondary-Return before nd's bindings, as
    /// a native field would; other secondary shortcuts still reach nd.
    pub(crate) fn drawn_claims_key(&self, keystroke: &Keystroke, window: &gpui::Window) -> bool {
        if self.dialog_claims_key(keystroke, window) {
            return true;
        }
        let m = &keystroke.modifiers;
        if (m.platform || m.control) && keystroke.key != "enter" {
            return false;
        }
        self.drawn.borrow().values().any(|d| {
            d.try_borrow().is_ok_and(|d| match &*d {
                Drawn::Palette(p) => p.model.is_presented() && p.focus.is_focused(window),
                _ => false,
            })
        })
    }

    /// Whether a presented palette covers `window`: its scrim takes every press, so nd's gesture
    /// recognizers under it see none.
    pub(crate) fn palette_covers(&self, window: WindowId) -> bool {
        if self.dialog_covers(window) {
            return true;
        }
        self.drawn.borrow().iter().any(|(node, d)| {
            d.try_borrow().is_ok_and(|d| matches!(&*d, Drawn::Palette(p) if p.model.is_presented()))
                && self.window_of(*node) == Some(window)
        })
    }

    fn drawn_of(&self, id: NodeId) -> Option<Rc<RefCell<Drawn>>> {
        self.drawn.borrow().get(&id).cloned()
    }

    /// Brings the drawn palettes and toast overlays of `window_id` in line with their props,
    /// before the window builds: focus moves into an opened palette and back out of a closed
    /// one, a shown toast starts its timeout. Needs GPUI's app, which building does not have.
    pub(crate) fn prepare_drawn(&self, window_id: nd_core::WindowId, window: &mut gpui::Window, cx: &mut gpui::App) {
        self.prepare_dialogs(window_id, window, cx);
        let nodes: Vec<(NodeId, nd_ui::backend::Props, bool)> = {
            let Ok(state) = self.state.try_borrow() else { return };
            state
                .nodes
                .iter()
                .filter(|(_, n)| kind_is::<CommandPaletteKind>(n) || kind_is::<ToastOverlayKind>(n))
                .map(|(id, n)| (*id, n.props.clone(), kind_is::<CommandPaletteKind>(n)))
                .collect()
        };
        for (id, props, palette) in nodes {
            if self.window_of(id) != Some(window_id) {
                continue;
            }
            if palette {
                let Some(props) = props.downcast_ref::<CommandPaletteProps>() else { continue };
                self.prepare_palette(id, props, window, cx);
            } else if let Some(props) = props.downcast_ref::<ToastOverlayProps>() {
                self.prepare_toasts(id, window_id, props, cx);
            }
        }
    }

    fn prepare_palette(&self, id: NodeId, props: &CommandPaletteProps, window: &mut gpui::Window, cx: &mut gpui::App) {
        let drawn = self
            .drawn
            .borrow_mut()
            .entry(id)
            .or_insert_with(|| {
                Rc::new(RefCell::new(Drawn::Palette(PaletteDrawn {
                    model: PaletteModel::default(),
                    focus: cx.focus_handle(),
                    previous: None,
                    applied: None,
                })))
            })
            .clone();
        let mut d = drawn.borrow_mut();
        let Drawn::Palette(p) = &mut *d else { return };
        if p.applied.as_ref() == Some(props) {
            return;
        }
        let was_open = p.applied.as_ref().is_some_and(|a| a.open);
        p.model.set_query(&props.query);
        p.model.set_items(&props.items);
        p.applied = Some(props.clone());
        if props.open && !was_open && p.model.present() {
            p.previous = window.focused(cx);
            p.focus.focus(window, cx);
        } else if !props.open && was_open {
            p.model.dismiss(true);
            if let Some(previous) = p.previous.take() {
                previous.focus(window, cx);
            }
        }
    }

    fn prepare_toasts(&self, id: NodeId, window_id: nd_core::WindowId, props: &ToastOverlayProps, cx: &mut gpui::App) {
        let drawn = self
            .drawn
            .borrow_mut()
            .entry(id)
            .or_insert_with(|| Rc::new(RefCell::new(Drawn::Toasts(ToastsDrawn::default()))))
            .clone();
        let changes = {
            let mut d = drawn.borrow_mut();
            let Drawn::Toasts(t) = &mut *d else { return };
            if t.applied.as_ref() == Some(props) {
                return;
            }
            t.applied = Some(props.clone());
            t.queue.sync(&props.toasts)
        };
        self.apply_toasts(id, window_id, &drawn, changes, cx);
    }

    fn apply_toasts(
        &self,
        id: NodeId,
        window_id: nd_core::WindowId,
        drawn: &Rc<RefCell<Drawn>>,
        changes: Vec<ToastChange>,
        cx: &mut gpui::App,
    ) {
        for change in &changes {
            let ToastChange::Show(toast) = change else { continue };
            let Some(timeout) = toast.timeout else { continue };
            let (backend, weak, toast_id) = (self.clone(), Rc::downgrade(drawn), toast.id.clone());
            cx.spawn(async move |cx| {
                cx.background_executor().timer(timeout).await;
                cx.update(|cx| {
                    let Some(drawn) = weak.upgrade() else { return };
                    let changes = match &mut *drawn.borrow_mut() {
                        Drawn::Toasts(t) => t.queue.expire(&toast_id),
                        _ => return,
                    };
                    backend.apply_toasts(id, window_id, &drawn, changes, cx);
                });
            })
            .detach();
        }
        let emit = self.state.borrow().events.emitter(id);
        if report_toasts(&emit, changes)
            && let Some(wake) = self.wake_fn()
        {
            wake(cx);
        }
        if let Some(handle) = self.window_handle(window_id) {
            let _ = handle.update(cx, |_, window, _| window.refresh());
        }
    }
}

impl Build<'_> {
    /// The drawn palette, source tree, toast overlay and banner; `None` for any other kind.
    pub(crate) fn nav(
        &self,
        id: NodeId,
        kind: WidgetKind,
        node: &Node,
        window: &mut gpui::Window,
    ) -> Option<AnyElement> {
        let theme = self.theme();
        let theme = &*theme;
        let marks = Marks::default();
        let element = self.nav_element(id, kind, node, theme, &marks, window);
        self.record_marks(id, marks.take());
        element
    }

    fn nav_element(
        &self,
        id: NodeId,
        kind: WidgetKind,
        node: &Node,
        theme: &Theme,
        marks: &Marks,
        window: &mut gpui::Window,
    ) -> Option<AnyElement> {
        let emit = self.backend.state.borrow().events.emitter(id);
        let wake = self.wake.clone();
        if is::<CommandPaletteKind>(kind) {
            let props = node.props.downcast_ref::<CommandPaletteProps>()?;
            let drawn = self.backend.drawn_of(id)?;
            let d = drawn.borrow();
            let Drawn::Palette(p) = &*d else { return None };
            let handler = drawn.clone();
            let on_input: OnInput<PaletteInput> = Rc::new(move |input, window, cx| {
                let (events, restore) = {
                    let mut d = handler.borrow_mut();
                    let Drawn::Palette(p) = &mut *d else { return };
                    let events = match input {
                        PaletteInput::Key(key) => p.model.key(key).unwrap_or_default(),
                        PaletteInput::Insert(text) => p.model.insert(&text),
                        PaletteInput::Backspace => p.model.backspace(),
                        PaletteInput::Click(i) => p.model.click(i),
                        PaletteInput::Cancel => p.model.dismiss(false).into_iter().collect(),
                    };
                    let restore = if p.model.is_presented() { None } else { p.previous.take() };
                    (events, restore)
                };
                if let Some(previous) = restore {
                    previous.focus(window, cx);
                }
                let reported = !events.is_empty();
                for event in events {
                    emit_palette_event(&emit, event);
                }
                if reported {
                    wake(cx);
                }
                window.refresh();
            });
            let overlay = palette_overlay(
                &p.model,
                props.placeholder.as_deref(),
                theme,
                window.viewport_size(),
                Some(&p.focus),
                Some(on_input),
                marks,
            );
            return Some(div().absolute().size_0().child(overlay).into_any_element());
        }
        if is::<ToastOverlayKind>(kind) {
            let drawn = self.backend.drawn_of(id)?;
            let current = match &*drawn.borrow() {
                Drawn::Toasts(t) => t.queue.current().cloned(),
                _ => None,
            };
            let backend = self.backend.clone();
            let window_id = self.window_id;
            let handler = drawn.clone();
            let on_input: OnInput<ToastInput> = Rc::new(move |input, _window, cx| {
                let changes = match &mut *handler.borrow_mut() {
                    Drawn::Toasts(t) => match input {
                        ToastInput::Body => t.queue.clicked(),
                        ToastInput::Action => t.queue.action(),
                    },
                    _ => return,
                };
                backend.apply_toasts(id, window_id, &handler, changes, cx);
            });
            let view = toast_view(current.as_ref(), theme, window.viewport_size(), true, Some(on_input), marks);
            return Some(div().absolute().size_0().child(view).into_any_element());
        }
        if is::<SourceTreeKind>(kind) {
            let props = node.props.downcast_ref::<SourceTreeProps>()?;
            let drawn = self
                .backend
                .drawn
                .borrow_mut()
                .entry(id)
                .or_insert_with(|| Rc::new(RefCell::new(Drawn::Tree(TreeDrawn::default()))))
                .clone();
            let hovered = match &*drawn.borrow() {
                Drawn::Tree(t) => t.hovered.clone(),
                _ => None,
            };
            let on_input: OnInput<TreeInput> = Rc::new(move |input, window, cx| {
                match input {
                    TreeInput::Select(id) => emit.emit(SourceSelected { id }),
                    TreeInput::Activate(id) => emit.emit(SourceActivated { id }),
                    TreeInput::Toggle(id, true) => emit.emit(SourceExpanded { id }),
                    TreeInput::Toggle(id, false) => emit.emit(SourceCollapsed { id }),
                    TreeInput::Action(id, action) => emit.emit(SourceActionClicked { id, action }),
                    TreeInput::Hover(row) => {
                        if let Drawn::Tree(t) = &mut *drawn.borrow_mut() {
                            t.hovered = row;
                        }
                        window.refresh();
                        return;
                    }
                }
                wake(cx);
            });
            let mut style = div();
            gpui::Refineable::refine(style.style(), &props.style);
            return Some(
                style
                    .child(source_tree_view(
                        id,
                        props,
                        hovered.as_ref(),
                        theme,
                        self.in_hidden_sidebar(node),
                        Some(on_input),
                        marks,
                    ))
                    .into_any_element(),
            );
        }
        if is::<BannerKind>(kind) {
            let props = node.props.downcast_ref::<BannerProps>()?;
            let on_action: OnAction = Rc::new(move |cx| {
                emit.emit(BannerActionClicked);
                wake(cx);
            });
            return Some(banner_view(id, props, theme, Some(on_action), marks));
        }
        None
    }
}

#[cfg(test)]
mod icon_slot_tests {
    use nd_components::Icon;

    use super::icon_glyph;
    use crate::forms::glyph::Glyph;

    #[test]
    fn a_slot_exists_only_for_an_icon_that_draws() {
        assert!(icon_glyph(None).is_none(), "no icon, no slot");
        assert!(icon_glyph(Some(Icon::Custom("never-registered"))).is_none(), "an icon with no shape, no slot");
        for icon in Icon::ALL {
            assert!(icon_glyph(Some(*icon)).is_some(), "{} draws", icon.lucide());
        }
    }

    #[test]
    fn a_toolbar_button_with_an_unknown_icon_name_draws_no_glyph() {
        assert!(Glyph::named("never-registered").is_none());
        assert!(Glyph::named("square-pen").is_some());
    }
}
