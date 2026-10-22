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
use std::sync::Arc;

use crate::forms::glyph::{Glyph, glyph};
use nd_components::{
    BANNER, BannerProps, CommandPaletteProps, DecodeRequest, FieldSelection, ImageFit, PALETTE, PaletteKey,
    PaletteModel, SourceIndex, SourceTreeProps, TOAST, Toast, ToastOverlayProps, ToastQueue,
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
    let preset = Preset::forced().unwrap_or(if cfg!(target_os = "windows") { Preset::Fluent } else { Preset::Adwaita });
    Theme::derive(preset, scheme, Palette::baked_for(preset, scheme), Metrics::preset(preset), 1.)
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
        // WinUI's list selection: SubtleFillColorSecondary under an accent pill, the text as is.
        let fluent = theme.preset == Preset::Fluent;
        let (bg, fg, secondary) = if highlighted && fluent {
            let subtle = with_alpha(theme.text.primary, if theme.is_dark() { 0.059 } else { 0.035 });
            (Some(subtle), theme.text.primary, theme.text.secondary)
        } else if highlighted {
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
        if highlighted && fluent {
            row = row.relative().child(crate::forms::text::selection_pill(theme.accent, px(16.)));
        }
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
    deferred(anchored().position(point(px(0.), px(0.))).child(crate::occlusion::Occluding::surface(scrim)))
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
    /// A press on a row that drags, with its title and where in the window it landed.
    Press(nd_ui::DragItem, SharedString, gpui::Point<Pixels>),
    /// A row's bounds in the window, as just laid out, by its id.
    Row(SharedString, gpui::Bounds<Pixels>),
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
    art: &HashMap<SharedString, Arc<gpui::RenderImage>>,
    marks: &Marks,
) -> gpui::Stateful<gpui::Div> {
    let (body, footnote) = (theme.type_scale.body, theme.type_scale.footnote);
    // WinUI's NavigationView: an icon per item, a subtle fill with an accent pill on the selected
    // one, and an icon-only rail when the pane is compact.
    let fluent = theme.preset == Preset::Fluent;
    let adwaita = theme.preset == Preset::Adwaita;
    let subtle = with_alpha(theme.text.primary, if theme.is_dark() { 0.06 } else { 0.04 });
    let (regular, semibold) = (FontWeight::NORMAL, FontWeight::SEMIBOLD);
    let mut list = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::Tree)
        .flex()
        .flex_col()
        .size_full()
        .overflow_y_scroll()
        // NavigationViewItem: 36 px rows 4 px in from the pane's edge, 4 px apart.
        .p(if fluent { px(4.) } else { theme.spacing.sm })
        .gap(if fluent {
            px(4.)
        } else if theme.preset == Preset::Adwaita {
            px(2.)
        } else {
            px(1.)
        })
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
        return list;
    }
    let index = SourceIndex::new(&props.nodes);
    let actions: HashMap<&SharedString, _> = props.actions.iter().map(|a| (&a.id, a)).collect();
    let hover_only = props.action_visibility == nd_components::ActionVisibility::Hover;
    let indent = theme.spacing.lg.as_f32();
    // Rows report where they are laid out, which a drag over the list places its slot by.
    let drags = props.drag_kind.is_some() || !props.accepts.is_empty();
    let report = |id: &SharedString| {
        let on_input = on_input.clone().filter(|_| drags)?;
        let id = id.clone();
        Some(
            gpui::canvas(
                move |bounds, window, cx| on_input(TreeInput::Row(id.clone(), bounds), window, cx),
                |_, (), _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
    };
    for (i, depth) in index.visible(&props.nodes) {
        let node = &props.nodes[i];
        if node.section && compact {
            continue;
        }
        if node.section && fluent {
            // NavigationViewItemHeader: body text, semibold, secondary, on a row's height.
            marks.text(&node.title, theme.text.secondary, &[], body, semibold);
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .h(px(FLUENT_ROW))
                    .px(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text.secondary)
                    .child(node.title.clone()),
            );
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
                    .relative()
                    .child(node.title.clone())
                    .children(report(&node.id)),
            );
            continue;
        }
        let selected = props.selected.as_ref() == Some(&node.id);
        let expandable = index.expandable(&props.nodes, i);
        // libadwaita's navigation sidebar selects with a tenth of the text colour and keeps the
        // text as it is (`.navigation-sidebar > row:selected`).
        let tinted = !fluent && !adwaita;
        let (fg, secondary) = if selected && tinted {
            (theme.accent_container.fg, theme.accent_container.fg.opacity(0.8))
        } else {
            (theme.text.primary, theme.text.secondary)
        };
        let selected_fill = if fluent {
            subtle
        } else if adwaita {
            with_alpha(theme.text.primary, theme.text.primary.a * 0.1)
        } else {
            theme.accent_container.bg
        };
        // Dim text on a selected libadwaita row darkens just enough to keep 4.5:1 on the tint.
        let secondary = if selected && adwaita {
            let under = nd_theme::color::over(selected_fill, theme.surface.sidebar);
            nd_theme::color::legible(secondary, &[under], 4.5)
        } else {
            secondary
        };
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
        // libadwaita's rows start 8 px in and keep no room for a disclosure they do not have.
        let disclosure_width = if adwaita && !expandable { px(0.) } else { theme.spacing.lg };
        let mut disclosure = div()
            .id(ElementId::Name(format!("{}-disclosure", node.id).into()))
            .flex_shrink_0()
            .w(disclosure_width)
            .text_color(secondary)
            .child(if expandable { if node.expanded { "\u{25be}" } else { "\u{25b8}" } } else { "" });
        if fluent {
            // WinUI puts the expand chevron at the row's trailing edge.
            let chevron = if cfg!(target_os = "windows") {
                crate::fluent_icons::glyph(if node.expanded { '\u{E70E}' } else { '\u{E70D}' }, px(12.), secondary)
                    .into_any_element()
            } else {
                let g = if node.expanded { Glyph::ChevronUp } else { Glyph::ChevronDown };
                glyph(g, px(12.), secondary, px(1.))
            };
            disclosure = div()
                .id(ElementId::Name(format!("{}-disclosure", node.id).into()))
                .flex_shrink_0()
                .ml(theme.spacing.sm)
                .children(expandable.then_some(chevron));
        }
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
        // libadwaita's sidebar rows: 36 px at the button radius.
        let row_height = if fluent {
            px(FLUENT_ROW)
        } else if adwaita {
            px(36.)
        } else {
            theme.control.regular
        };
        // At the item's indentation: a child's pill sits in from its parent's.
        let pill = (fluent && selected).then(|| {
            div()
                .absolute()
                .left(px(32. * depth as f32))
                .top((row_height - px(16.)) / 2.)
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
            .min_h(row_height)
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
            // WinUI: the icon 12 px in, the label 16 px past it, children indented to the label.
            // libadwaita: 8 px in from either edge.
            let (lead, indent, trail, icon_gap) = if fluent {
                (px(12.), px(32.), px(14.), px(16.))
            } else if adwaita {
                (px(8.), px(indent), px(8.), theme.spacing.md)
            } else {
                (px(0.), px(indent), theme.spacing.sm, theme.spacing.md)
            };
            let (leading, trailing) = match fluent {
                true => (None, expandable.then_some(disclosure)),
                false => (Some(disclosure), None),
            };
            row = row
                .pl(lead + indent * depth as f32)
                .pr(trail)
                .children(leading)
                .children(match (art.get(&node.id), icon) {
                    // A cover in the icon's place, at the sidebar's icon size with Music's corners.
                    (Some(image), _) => {
                        let gap = if fluent { icon_gap } else { theme.spacing.md };
                        let mut corners = div().rounded(px(4.));
                        let corners = corners.style().clone();
                        Some(
                            div()
                                .flex_none()
                                .size(px(nd_components::SOURCE_ART))
                                .mr(gap)
                                .child(crate::media::image::picture(image.clone(), ImageFit::Cover, &corners))
                                .into_any_element(),
                        )
                    }
                    (None, Some(icon)) => Some(icon.mr(icon_gap).into_any_element()),
                    // A symbolic icon in the text colour, 12 px before the title on libadwaita.
                    (None, None) => icon_glyph(node.icon).map(|g| {
                        let (gap, ink) = if adwaita { (theme.spacing.md, fg) } else { (theme.spacing.sm, secondary) };
                        div().flex_shrink_0().pr(gap).child(glyph(g, px(16.), ink, px(1.5))).into_any_element()
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
                }))
                .children(trailing);
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
            if adwaita {
                let hover = with_alpha(theme.text.primary, theme.text.primary.a * 0.13);
                row = row.hover(move |s| s.bg(hover));
            }
        } else {
            let hover = if fluent {
                subtle
            } else if adwaita {
                with_alpha(theme.text.primary, theme.text.primary.a * 0.07)
            } else {
                theme.hover(theme.surface.sidebar)
            };
            row = row.hover(move |s| s.bg(hover));
        }
        if fluent {
            // SubtleFillColorTertiary while pressed.
            let pressed = with_alpha(theme.text.primary, if theme.is_dark() { 0.039 } else { 0.024 });
            row = row.active(move |s| s.bg(pressed));
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
            if let Some(item) = props.drag_item(i) {
                let (on_press, title) = (on_input.clone(), node.title.clone());
                row = row.on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    on_press(TreeInput::Press(item.clone(), title.clone(), event.position), window, cx);
                });
            }
            if hover_only {
                row = row.on_hover(move |inside, window, cx| {
                    on_input(TreeInput::Hover(inside.then(|| key.clone())), window, cx);
                });
            }
        }
        list = list.child(row.children(report(&node.id)));
    }
    list
}

/// Window-wide pointer listeners for a press on a row of a tree whose rows drag: past
/// [`DRAG_START`] the press becomes an nd drag ([`nd_ui::DragSource`]), which follows the pointer
/// out of the list and the window and ends on the release.
fn row_drag_listener(drawn: &Rc<RefCell<Drawn>>, emit: &Emit, wake: &crate::backend::Wake) -> AnyElement {
    let (moved, released) = (drawn.clone(), drawn.clone());
    let (emit_move, emit_up) = (emit.clone(), emit.clone());
    let (wake_move, wake_up) = (wake.clone(), wake.clone());
    gpui::canvas(
        |_, _, _| (),
        move |_, (), window, _| {
            let (drawn, emit, wake) = (moved.clone(), emit_move.clone(), wake_move.clone());
            window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, window, cx| {
                if phase != gpui::DispatchPhase::Capture {
                    return;
                }
                let Drawn::Tree(t) = &mut *drawn.borrow_mut() else { return };
                let Some(press) = t.press.as_mut() else { return };
                if event.pressed_button != Some(MouseButton::Left) {
                    if press.dragging {
                        emit.emit(drag_source(&press.item, nd_ui::GesturePhase::Ended, press.at));
                        wake(cx);
                    }
                    t.press = None;
                    t.slot = None;
                    window.refresh();
                    return;
                }
                press.at = event.position;
                let moved = press.at - press.start;
                if !press.dragging && moved.x.as_f32().hypot(moved.y.as_f32()) < DRAG_START {
                    return;
                }
                if !press.dragging {
                    press.dragging = true;
                    emit.emit(drag_source(&press.item, nd_ui::GesturePhase::Began, press.start));
                }
                emit.emit(drag_source(&press.item, nd_ui::GesturePhase::Changed, press.at));
                wake(cx);
                window.refresh();
            });
            let (drawn, emit, wake) = (released.clone(), emit_up.clone(), wake_up.clone());
            window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, window, cx| {
                if phase != gpui::DispatchPhase::Capture || event.button != MouseButton::Left {
                    return;
                }
                let Drawn::Tree(t) = &mut *drawn.borrow_mut() else { return };
                let Some(press) = t.press.take() else { return };
                t.slot = None;
                if press.dragging {
                    emit.emit(drag_source(&press.item, nd_ui::GesturePhase::Ended, event.position));
                    wake(cx);
                    // The release ends the drag, not a click on the row under it.
                    cx.stop_propagation();
                    window.refresh();
                }
            });
        },
    )
    .absolute()
    .size_0()
    .into_any_element()
}

/// NavigationViewItem's height in the pane.
const FLUENT_ROW: f32 = 36.;

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
    if theme.preset == Preset::Adwaita {
        return adw_toast(toast, theme, viewport, animate, on_input, marks);
    }
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
        .child(crate::occlusion::Occluding::passive(body));
    deferred(anchored().position(point(px(0.), px(0.))).child(layer))
        .with_priority(crate::overlay::TOAST_PRIORITY)
        .into_any_element()
}

/// AdwToast (`_misc.scss`, `adw-toast-overlay.c`): a pill of the OSD grey with white text, a
/// bold title, the button as a regular button and a flat close button, 12 px from the window's
/// sides and 24 px from its bottom. It slides up from below the window over 300 ms on
/// libadwaita's ease.
fn adw_toast(
    toast: &Toast,
    theme: &Theme,
    viewport: gpui::Size<gpui::Pixels>,
    animate: bool,
    on_input: Option<OnInput<ToastInput>>,
    marks: &Marks,
) -> AnyElement {
    const HEIGHT: f32 = 46.;
    const BOTTOM: f32 = 24.;
    let (bg, ink) = (theme.surface.osd, theme.text.on_osd);
    let tint = |share: f32| with_alpha(ink, ink.a * share);
    let under = [bg];
    marks.fill("toast", bg, &[]);
    marks.text(&toast.title, ink, &under, theme.type_scale.body, FontWeight::BOLD);
    let button = |id: &'static str, rest: Hsla, hover: Hsla, active: Hsla| {
        div()
            .id(id)
            .role(gpui::Role::Button)
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .h(px(34.))
            .rounded(theme.radius.md)
            .bg(rest)
            .hover(move |s| s.bg(hover))
            .active(move |s| s.bg(active))
    };
    let mut body = div()
        .id(ElementId::Name(format!("nd-toast-{}", toast.id).into()))
        .role(gpui::Role::Status)
        .aria_label(toast.title.clone())
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .h(px(HEIGHT))
        .max_w(px((viewport.width.as_f32() - 24.).max(0.)))
        .p(px(6.))
        .pl(px(12.))
        .rounded_full()
        .bg(bg)
        .shadow(vec![
            BoxShadow {
                color: gpui::hsla(240. / 360., 1., 0.012, 0.07),
                offset: point(px(0.), px(1.)),
                blur_radius: px(3.),
                spread_radius: px(1.),
                inset: false,
            },
            BoxShadow {
                color: gpui::hsla(240. / 360., 1., 0.012, 0.03),
                offset: point(px(0.), px(2.)),
                blur_radius: px(6.),
                spread_radius: px(2.),
                inset: false,
            },
        ])
        .text_size(theme.type_scale.body)
        .text_color(ink)
        .occlude()
        .child(div().min_w_0().flex_1().px(px(6.)).truncate().font_weight(FontWeight::BOLD).child(toast.title.clone()));
    if let Some(label) = &toast.action {
        marks.fill("toast button", tint(0.1), &under);
        marks.text(label, ink, &[bg, tint(0.1)], theme.type_scale.body, FontWeight::BOLD);
        let mut action = button("nd-toast-action", tint(0.1), tint(0.15), tint(0.3))
            .aria_label(label.clone())
            .px(px(17.))
            .font_weight(FontWeight::BOLD)
            .child(label.clone());
        if let Some(on_input) = on_input.clone() {
            action = action.on_click(move |_, window, cx| {
                cx.stop_propagation();
                on_input(ToastInput::Action, window, cx);
            });
        }
        body = body.child(action);
    }
    marks.fill("toast close", ink, &under);
    let mut close = button("nd-toast-close", gpui::transparent_black(), tint(0.07), tint(0.16))
        .aria_label("Dismiss")
        .w(px(34.))
        .rounded_full()
        .child(glyph(Glyph::Clear, px(16.), ink, px(1.75)));
    if let Some(on_input) = on_input.clone() {
        close = close.on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_input(ToastInput::Body, window, cx);
        });
    }
    body = body.child(close);
    if let Some(on_input) = on_input {
        body = body.on_click(move |_, window, cx| on_input(ToastInput::Body, window, cx));
    }
    let body: AnyElement = if animate {
        let rise = HEIGHT + BOTTOM;
        body.with_animation(
            ElementId::Name(format!("nd-toast-enter-{}", toast.id).into()),
            Animation::new(std::time::Duration::from_millis(300))
                .with_easing(|t| crate::forms::look::cubic_bezier([0.25, 0.1, 0.25, 1.], t)),
            move |el, delta| el.mt(px(rise * (1. - delta))),
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
        .px(px(12.))
        .pb(px(BOTTOM))
        .child(crate::occlusion::Occluding::passive(body));
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
    if theme.preset == Preset::Fluent {
        return info_bar(id, props, theme, on_action, marks);
    }
    if theme.preset == Preset::Adwaita {
        return adw_banner(id, props, theme, on_action, marks);
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
    bar.into_any_element()
}

/// The banner as WinUI's informational InfoBar (InfoBar_themeresources.xaml): a 48 px card in
/// SystemFillColorAttentionBackground with a CardStrokeColorDefault outline, the accent info
/// badge, the message and a standard button. It sits inset in the content, where libadwaita's
/// banner spans it edge to edge, which also keeps it clear of the content layer's rounded corner.
fn info_bar(id: NodeId, props: &BannerProps, theme: &Theme, on_action: Option<OnAction>, marks: &Marks) -> AnyElement {
    let dark = theme.is_dark();
    let look = crate::forms::look::FormLook::fluent(theme);
    let bg: Hsla = if dark { with_alpha(gpui::white(), 0.031) } else { gpui::rgba(0xf6f6f680).into() };
    let stroke = with_alpha(gpui::black(), if dark { 0.098 } else { 0.059 });
    let inverse = if dark { with_alpha(gpui::black(), 0.894) } else { gpui::white() };
    let under = [theme.surface.layer];
    marks.fill("banner", bg, &under);
    marks.text(&props.title, theme.text.primary, &[under[0], bg], theme.type_scale.body, FontWeight::NORMAL);
    // InfoBarIconBackgroundGlyph under InfoBarInformationalIconGlyph, 16 px.
    let badge = if cfg!(target_os = "windows") {
        div()
            .relative()
            .size(px(16.))
            .child(crate::fluent_icons::glyph('\u{F136}', px(16.), theme.accent))
            .child(crate::fluent_icons::glyph('\u{F13F}', px(16.), inverse).absolute().top_0().left_0())
    } else {
        div()
            .size(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(theme.accent)
            .text_color(inverse)
            .text_size(px(11.))
            .font_weight(FontWeight::BOLD)
            .child("i")
    };
    let mut bar = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::Status)
        .aria_label(props.title.clone())
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .min_h(px(48.))
        .pl(px(16.))
        .pr(px(16.))
        .rounded(theme.radius.md)
        .bg(bg)
        .border_1()
        .border_color(stroke)
        .text_color(theme.text.primary)
        .text_size(theme.type_scale.body)
        .child(badge.flex_none().mr(px(14.)))
        .child(div().flex_1().min_w_0().truncate().child(props.title.clone()));
    if let Some(label) = &props.action {
        marks.fill("banner button", look.fill, &[under[0], bg]);
        marks.text(label, look.text, &[under[0], bg, look.fill], theme.type_scale.body, FontWeight::NORMAL);
        let edge = crate::forms::toggle::elevation_edge(look, look.radius);
        let mut button = div()
            .id("nd-banner-action")
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .relative()
            .flex()
            .flex_shrink_0()
            .items_center()
            .ml(px(16.))
            .my(px(8.))
            .h(look.control_height)
            .px(look.padding_x)
            .rounded(look.radius)
            .bg(look.fill)
            .border_1()
            .border_color(look.border)
            .children(edge)
            .text_color(look.text)
            .hover(move |s| s.bg(look.fill_hover))
            .active(move |s| s.bg(look.fill_active))
            .child(label.clone());
        if let Some(on_action) = on_action {
            button = button.on_click(move |_, _, cx| on_action(cx));
        }
        bar = bar.child(button);
    }
    div().w_full().p(theme.spacing.md).child(bar).into_any_element()
}

/// AdwBanner: a neutral strip (libadwaita's banner grey at 30% over the window, here the text
/// colour at the same weight), the title bold and centred, the button a regular button, 6 px
/// around.
fn adw_banner(
    id: NodeId,
    props: &BannerProps,
    theme: &Theme,
    on_action: Option<OnAction>,
    marks: &Marks,
) -> AnyElement {
    let ink = theme.text.primary;
    let bg = with_alpha(ink, ink.a * if theme.is_dark() { 0.12 } else { 0.15 });
    let button = with_alpha(ink, ink.a * 0.1);
    marks.fill("banner", bg, &[]);
    marks.text(&props.title, ink, &[bg], theme.type_scale.body, FontWeight::BOLD);
    let mut bar = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::Status)
        .aria_label(props.title.clone())
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .min_h(px(46.))
        .gap(px(6.))
        .p(px(6.))
        .bg(bg)
        .text_color(ink)
        .text_size(theme.type_scale.body)
        .font_weight(FontWeight::BOLD)
        .child(div().flex_1().min_w_0().px(px(6.)).text_center().truncate().child(props.title.clone()));
    if let Some(label) = &props.action {
        marks.fill("banner button", button, &[bg]);
        marks.text(label, ink, &[bg, button], theme.type_scale.body, FontWeight::BOLD);
        let (hover, active) = (with_alpha(ink, ink.a * 0.15), with_alpha(ink, ink.a * 0.3));
        let mut b = div()
            .id("nd-banner-action")
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .flex()
            .flex_shrink_0()
            .items_center()
            .h(px(34.))
            .px(px(17.))
            .rounded(theme.radius.md)
            .bg(button)
            .hover(move |s| s.bg(hover))
            .active(move |s| s.bg(active))
            .child(label.clone());
        if let Some(on_action) = on_action {
            b = b.on_click(move |_, _, cx| on_action(cx));
        }
        bar = bar.child(b);
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
    /// A press on a row, which turns into a drag once the pointer moves [`DRAG_START`].
    pub press: Option<RowPress>,
    /// Each row's bounds in the window as last laid out, by id.
    pub rows: HashMap<SharedString, gpui::Bounds<Pixels>>,
    /// The line marking where a drag over the list lands: its left end and width.
    pub slot: Option<(gpui::Point<Pixels>, Pixels)>,
    /// Each row's artwork load, by id, while the row has artwork.
    pub art: HashMap<SharedString, Rc<RefCell<crate::media::image::Picture>>>,
}

pub(crate) struct RowPress {
    item: nd_ui::DragItem,
    title: SharedString,
    start: gpui::Point<Pixels>,
    at: gpui::Point<Pixels>,
    /// The press point's offset into its row, and the row's width.
    grab: gpui::Point<Pixels>,
    width: Pixels,
    dragging: bool,
}

/// How far a press moves before it drags, as a window tab's does.
const DRAG_START: f32 = 4.;

fn drag_source(item: &nd_ui::DragItem, phase: nd_ui::GesturePhase, at: gpui::Point<Pixels>) -> nd_ui::DragSource {
    let at = nd_ui::DragPoint::Window(nd_core::Point { x: at.x.as_f32(), y: at.y.as_f32() });
    nd_ui::DragSource { item: item.clone(), phase, at, native: false }
}

impl TreeDrawn {
    /// Where a drag at `at` (window coordinates) lands among `props`' rows, with the slot line
    /// placed there.
    pub(crate) fn drop_slot(&mut self, props: &SourceTreeProps, at: nd_core::Point) -> nd_ui::DropSlot {
        let index = SourceIndex::new(&props.nodes);
        let visible = index.visible(&props.nodes);
        let y = px(at.y);
        let rows: Vec<(usize, gpui::Bounds<Pixels>)> =
            visible.iter().filter_map(|(i, _)| Some((*i, *self.rows.get(&props.nodes[*i].id)?))).collect();
        let over = rows.iter().find(|(_, b)| y >= b.top() && y < b.bottom());
        let (row, upper, line) = match over {
            Some((i, b)) => {
                let upper = y < b.center().y;
                let below = !upper || props.nodes[*i].section;
                let line_y = if below { b.bottom() } else { b.top() };
                (visible.iter().position(|(v, _)| v == i), upper, Some((point(b.left(), line_y), b.size.width)))
            }
            None => match rows.first() {
                Some((_, first)) if y < first.top() => {
                    (Some(0), true, Some((point(first.left(), first.top()), first.size.width)))
                }
                _ => (None, false, rows.last().map(|(_, b)| (point(b.left(), b.bottom()), b.size.width))),
            },
        };
        self.slot = line;
        index.drop_slot(&props.nodes, &visible, row, upper)
    }
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

    pub(crate) fn drawn_of(&self, id: NodeId) -> Option<Rc<RefCell<Drawn>>> {
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
    /// The source tree's row artwork decoded so far, by row id, asking the pipeline for the rest.
    /// A row whose picture has not landed draws its icon.
    fn source_art(
        &self,
        drawn: &Rc<RefCell<Drawn>>,
        props: &SourceTreeProps,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> HashMap<SharedString, Arc<gpui::RenderImage>> {
        use crate::media::image::Picture;
        let Drawn::Tree(tree) = &mut *drawn.borrow_mut() else { return HashMap::new() };
        let side = nd_components::SOURCE_ART;
        let app = self.backend.state.borrow().app.clone();
        let wanted: HashMap<&SharedString, &nd_components::ImageSource> =
            props.nodes.iter().filter(|n| !n.section).filter_map(|n| n.artwork.as_ref().map(|a| (&n.id, a))).collect();
        tree.art.retain(|id, _| wanted.contains_key(id));
        let mut shown = HashMap::new();
        for (id, source) in wanted {
            let picture =
                tree.art.entry(id.clone()).or_insert_with(|| Rc::new(RefCell::new(Picture::default()))).clone();
            picture.borrow_mut().attach(None, app.clone());
            let request = DecodeRequest::at(source.clone(), side, side, window.scale_factor(), ImageFit::Cover);
            Picture::want(&picture, Some(request), window, cx);
            if let Some(image) = Picture::current(&picture, false, Some(1.), window, cx) {
                shown.insert(id.clone(), image);
            }
        }
        shown
    }

    /// The drawn palette, source tree, toast overlay and banner; `None` for any other kind.
    pub(crate) fn nav(
        &self,
        id: NodeId,
        kind: WidgetKind,
        node: &Node,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> Option<AnyElement> {
        let theme = self.theme();
        let theme = &*theme;
        let marks = Marks::default();
        let element = self.nav_element(id, kind, node, theme, &marks, window, cx);
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
        cx: &mut gpui::App,
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
            let (hovered, slot, preview) = match &*drawn.borrow() {
                Drawn::Tree(t) => (
                    t.hovered.clone(),
                    t.slot,
                    t.press.as_ref().filter(|p| p.dragging).map(|p| (p.title.clone(), p.at - p.grab, p.width)),
                ),
                _ => (None, None, None),
            };
            let listener = props.drag_kind.is_some().then(|| row_drag_listener(&drawn, &emit, &wake));
            let art = self.source_art(&drawn, props, window, cx);
            let tree = drawn.clone();
            let on_input: OnInput<TreeInput> = Rc::new(move |input, window, cx| {
                match input {
                    TreeInput::Press(item, title, at) => {
                        if let Drawn::Tree(t) = &mut *tree.borrow_mut() {
                            let row = t.rows.values().copied().find(|b| b.contains(&at));
                            let origin = row.map_or(at, |b| b.origin);
                            t.press = Some(RowPress {
                                item,
                                title,
                                start: at,
                                at,
                                grab: at - origin,
                                width: row.map_or(px(200.), |b| b.size.width),
                                dragging: false,
                            });
                        }
                        return;
                    }
                    TreeInput::Row(id, bounds) => {
                        if let Drawn::Tree(t) = &mut *tree.borrow_mut() {
                            t.rows.insert(id, bounds);
                        }
                        return;
                    }
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
            let tree = source_tree_view(
                id,
                props,
                hovered.as_ref(),
                theme,
                self.in_hidden_sidebar(node),
                Some(on_input),
                &art,
                marks,
            );
            return Some(
                style
                    .child(self.scrolled(id, tree, crate::scrollbar::Fit::Fill, window))
                    .children(slot.map(|(at, width)| {
                        let line = div().w(width).h(px(2.)).rounded(px(1.)).bg(theme.accent);
                        deferred(anchored().position(point(at.x, at.y - px(1.))).child(line))
                            .with_priority(crate::overlay::PANEL_PRIORITY)
                    }))
                    .children(
                        preview
                            .map(|(title, at, width)| crate::tab_bar::drag_preview(&title, theme, at, width.as_f32())),
                    )
                    .children(listener)
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
