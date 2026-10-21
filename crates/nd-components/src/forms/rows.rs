//! Settings rows and form layout, composed from the controls (parity with the zig `row`,
//! `switchrow`, `settingsgroup`, `clamp` widgets and the `ui` package's `FormField` and
//! `ButtonGroup`). Each is built from intents the backends already map, so a row inside
//! `form()` is a System Settings row on macOS and a libadwaita-style row on GPUI.

use nd_gpui::gpui::Refineable;
use nd_ui::prelude::*;
use nd_ui::{Bezel, StyleRefinement};

use super::display::{LabelTone, TextStyle, label};
use super::toggle::{Switch, Toggled, switch};
use crate::controls::ButtonExt;
use crate::form::{FormRow, FormSection, form_row, form_section};
use crate::icon::Icon;
use crate::overrides::OnPlatform;

/// A titled group of rows; `form_section` under its libadwaita name.
#[track_caller]
pub fn settings_group(title: impl Into<SharedString>) -> FormSection {
    form_section().title(title)
}

/// A row with a title and optional subtitle, holding one trailing control (`.child(..)`).
#[track_caller]
pub fn row(title: impl Into<SharedString>) -> FormRow {
    form_row(title)
}

/// The user activated a navigation row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowActivated;

/// A row whose trailing control is a switch. The row's title names the switch for
/// accessibility.
pub struct SwitchRow {
    row: FormRow,
    switch: Switch,
}

/// A titled row with a switch, controlled like [`switch`].
#[track_caller]
pub fn switch_row(id: impl Into<SharedString>, title: impl Into<SharedString>) -> SwitchRow {
    let id = id.into();
    let title = title.into();
    SwitchRow { row: form_row(title.clone()).id(format!("{id}-row")), switch: switch(id).label(title) }
}

impl SwitchRow {
    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.row = self.row.help(subtitle);
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.switch = self.switch.checked(checked);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.switch = self.switch.disabled(disabled);
        self
    }

    pub fn on_toggle(mut self, f: impl Fn(&Toggled, &mut Window, &mut App) + 'static) -> Self {
        self.switch = self.switch.on_toggle(f);
        self
    }
}

impl IntoElement for SwitchRow {
    fn into_any_element(self) -> AnyElement {
        self.row.child(self.switch).into_any_element()
    }
}

/// A row that navigates somewhere: a disclosure chevron as its control, firing
/// [`RowActivated`].
pub fn navigation_row(
    id: impl Into<SharedString>,
    title: impl Into<SharedString>,
    on_activate: impl Fn(&RowActivated, &mut Window, &mut App) + 'static,
) -> FormRow {
    let id = id.into();
    let title = title.into();
    form_row(title.clone()).id(format!("{id}-row")).child(
        button(id, "")
            .icon(Icon::ChevronRight)
            .bezel(Bezel::Borderless)
            .tooltip(title)
            .on_click(move |_: &ClickEvent, window, cx| on_activate(&RowActivated, window, cx)),
    )
}

/// Content at a readable width: up to `maximum` wide (600 points by default, libadwaita's
/// `AdwClamp`), centred in wider space.
pub struct Clamp(Div);

#[track_caller]
pub fn clamp() -> Clamp {
    Clamp(v_flex().w_full().max_w(px(600.)).mx_auto())
}

impl Clamp {
    pub fn maximum(mut self, maximum: Pixels) -> Self {
        self.0 = self.0.max_w(maximum);
        self
    }
}

impl Styled for Clamp {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl ParentElement for Clamp {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.0.extend(elements);
    }
}

impl IntoElement for Clamp {
    fn into_any_element(self) -> AnyElement {
        self.0.into_any_element()
    }
}

/// A labelled control stacked vertically with help and error text, for forms outside a
/// grouped `form()` (the `ui` package's `FormField`).
pub struct FormField {
    label: SharedString,
    help: Option<SharedString>,
    error: Option<SharedString>,
    control: Option<AnyElement>,
    style: StyleRefinement,
}

pub fn form_field(label: impl Into<SharedString>) -> FormField {
    FormField { label: label.into(), help: None, error: None, control: None, style: StyleRefinement::default() }
}

impl FormField {
    pub fn help(mut self, help: impl Into<SharedString>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Shown in place of the help text while set.
    pub fn error(mut self, error: Option<impl Into<SharedString>>) -> Self {
        self.error = error.map(Into::into);
        self
    }

    pub fn control(mut self, control: impl IntoElement) -> Self {
        self.control = Some(control.into_any_element());
        self
    }
}

impl Styled for FormField {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl IntoElement for FormField {
    fn into_any_element(self) -> AnyElement {
        let mut column = v_flex().gap_1();
        column.style().refine(&self.style);
        let note = match (self.error, self.help) {
            (Some(error), _) => Some(label(error).text_style(TextStyle::Footnote).tone(LabelTone::Danger)),
            (None, Some(help)) => Some(label(help).text_style(TextStyle::Footnote).tone(LabelTone::Secondary)),
            (None, None) => None,
        };
        column
            // WinUI's TextBox header: Body, regular, 8 px above the control.
            .child(
                label(self.label).text_style(TextStyle::Headline).on_windows(|l| l.text_style(TextStyle::Body).mb_1()),
            )
            .children(self.control)
            .children(note)
            .into_any_element()
    }
}

/// Buttons joined into one control (the `ui` package's `ButtonGroup`).
pub struct ButtonGroup(Div);

#[track_caller]
pub fn button_group() -> ButtonGroup {
    ButtonGroup(h_flex().gap_1())
}

impl ParentElement for ButtonGroup {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.0.extend(elements);
    }
}

impl Styled for ButtonGroup {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl IntoElement for ButtonGroup {
    fn into_any_element(self) -> AnyElement {
        self.0.into_any_element()
    }
}

/// A sheet's content (`window.open_sheet`): its title, the body, and a footer of buttons at the
/// trailing edge with the default button last, as macOS sheets and libadwaita dialogs lay them.
pub struct SheetLayout {
    title: SharedString,
    body: Vec<AnyElement>,
    footer: Vec<AnyElement>,
    style: StyleRefinement,
}

pub fn sheet_layout(title: impl Into<SharedString>) -> SheetLayout {
    SheetLayout { title: title.into(), body: Vec::new(), footer: Vec::new(), style: StyleRefinement::default() }
}

impl SheetLayout {
    /// Adds a footer button; add the default action last.
    pub fn footer(mut self, button: impl IntoElement) -> Self {
        self.footer.push(button.into_any_element());
        self
    }
}

impl ParentElement for SheetLayout {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.body.extend(elements);
    }
}

impl Styled for SheetLayout {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl IntoElement for SheetLayout {
    fn into_any_element(self) -> AnyElement {
        let mut column = v_flex().size_full().p_5().gap_4();
        column.style().refine(&self.style);
        column
            .child(label(self.title).text_style(TextStyle::Headline))
            .child(v_flex().flex_1().min_h_0().children(self.body))
            .when(!self.footer.is_empty(), |c| c.child(h_flex().justify_end().gap_2().children(self.footer)))
            .into_any_element()
    }
}
