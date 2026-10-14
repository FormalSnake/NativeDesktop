//! Forms and controls (M6): text and search fields, text area, checkbox, switch, radio group,
//! segmented control, slider, stepper, pop-up and combo box, date and colour pickers, progress,
//! labels and links, settings rows, and `settings_form::<T>()`.
//!
//! Each control is a widget kind whose props describe it to the semantic tree and answer
//! automation's `setValue`/`type`/`click` with the event the control would emit
//! ([`WidgetSemantics`]). Values are controlled: the app owns them, the control reports user
//! changes as events, and a backend applies a value through `Synced`, so feeding a reported or
//! normalised value back sets nothing (ARCHITECTURE.md 3.13).

mod choice;
mod date;
mod display;
mod rows;
#[cfg(feature = "settings")]
pub mod settings;
mod text;
mod toggle;
mod value;

pub use choice::{Select, SelectChanged, SelectKind, SelectProps, option_index, radio_group, segmented, select};
pub use date::{
    CivilDateTime, ColorChanged, ColorWell, ColorWellKind, ColorWellProps, DateChanged, DatePicker, DatePickerKind,
    DatePickerMode, DatePickerProps, DatePickerStyle, color_well, date_picker,
};
pub use display::{
    Label, LabelKind, LabelProps, LabelTone, Link, LinkActivated, LinkKind, LinkProps, Progress, ProgressKind,
    ProgressProps, ProgressStyle, TextStyle, label, link, progress, spinner,
};
pub use rows::{
    ButtonGroup, Clamp, FormField, RowActivated, SheetLayout, SwitchRow, button_group, clamp, form_field,
    navigation_row, row, settings_group, sheet_layout, switch_row,
};
pub use text::{
    ComboBox, ComboBoxKind, ComboBoxProps, FieldIcon, FieldIconClicked, FieldStyle, FilesDropped, IconEdge,
    ImagesPasted, PastedImage, TextArea, TextAreaKind, TextAreaProps, TextChanged, TextEvent, TextField,
    TextFieldKind, TextFieldProps, TextFieldState, TextInput, TextProps, TextSubmitted, combo_box, search_field,
    secure_field, text_area, text_field,
};
pub use toggle::{Checkbox, CheckboxKind, CheckboxProps, Switch, SwitchKind, SwitchProps, Toggled, checkbox, switch};
pub use value::{
    Slider, SliderChanged, SliderKind, SliderProps, Stepper, StepperChanged, StepperKind, StepperProps, number_text,
    slider, stepper,
};

use nd_ui::semantics::{ActionError, Response};

/// What every control carries besides its own props.
pub(crate) fn not_taken(what: &str) -> Response {
    Err(ActionError::NotActionable(what.to_owned()))
}

/// `Some(Err(..))` for a disabled control, so automation reports it instead of acting.
pub(crate) fn guard(enabled: bool) -> Option<Response> {
    (!enabled).then(|| not_taken("the control is disabled"))
}

/// Leaf builders: style, dispatch, `disabled`, `tooltip` and `props`, with the element's style
/// copied into the props at build time so the backend sees it as a prop change.
macro_rules! leaf {
    ($ty:ident, $props:ty) => {
        impl nd_ui::Styled for $ty {
            fn style(&mut self) -> &mut nd_ui::StyleRefinement {
                self.0.style()
            }
        }

        impl nd_ui::FluentBuilder for $ty {}

        impl nd_ui::InteractiveElement for $ty {
            fn dispatch_props(&mut self) -> &mut nd_ui::DispatchProps {
                self.0.dispatch_props()
            }
        }

        impl nd_ui::IntoElement for $ty {
            fn into_any_element(mut self) -> nd_ui::AnyElement {
                let style = nd_ui::Styled::style(&mut self.0).clone();
                self.0.props_mut().style = style;
                self.0.into_any_element()
            }
        }

        impl $ty {
            /// Greys the control out; it takes no input and automation reports it disabled.
            pub fn disabled(mut self, disabled: bool) -> Self {
                self.0.props_mut().enabled = !disabled;
                self
            }

            pub fn tooltip(mut self, tooltip: impl Into<nd_ui::prelude::SharedString>) -> Self {
                self.0.props_mut().tooltip = Some(tooltip.into());
                self
            }

            pub fn props(&self) -> &$props {
                self.0.props()
            }
        }
    };
}

pub(crate) use leaf;
