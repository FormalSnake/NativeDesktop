//! `settings_form::<T>()`: a settings struct's schema ([`SettingsSchema`]) as native settings
//! rows. Doc comments become help text, ranges become sliders or steppers, enums pickers,
//! booleans switches; a key set by a Nix-managed file shows read-only with its option path.
//! Edits go through [`SettingsHandle::set`], which keeps the file's comments.

use nd_settings::{Field, FieldKind, KeyState, Reload, Settings, SettingsHandle, SettingsSchema};
use nd_ui::deferred;
use nd_ui::prelude::*;
use serde_json::Value;

use super::display::label;
use super::text::{TextChanged, text_field};
use super::toggle::{Toggled, switch};
use super::value::{SliderChanged, StepperChanged, slider, stepper};
use super::{SelectChanged, select};
use crate::form::{FormRow, FormSection, form, form_row, form_section};

/// The settings a [`settings_form`] edits, as an app global. The app (or `nd`'s
/// `Application::settings`) provides it once at launch.
pub struct SettingsSource<T: Settings>(pub SettingsHandle<T>);

/// Makes `handle` the settings every `settings_form::<T>()` shows and writes.
pub fn provide<T: Settings>(cx: &mut App, handle: SettingsHandle<T>) {
    cx.set_global(SettingsSource(handle));
}

/// The settings form for `T`. Without a provided handle it shows the defaults, read-only.
pub fn settings_form<T: Settings>() -> AnyElement {
    deferred(|_window, cx| {
        let handle = cx.try_global::<SettingsSource<T>>().map(|s| s.0.clone());
        let schema = handle.as_ref().map_or_else(SettingsSchema::of::<T>, |h| h.schema().clone());
        let values = match &handle {
            Some(h) => serde_json::to_value(&*h.get()),
            None => serde_json::to_value(T::default()),
        }
        .unwrap_or(Value::Null);
        build::<T>(&schema, &values, handle.as_ref())
    })
}

fn pointer(path: &str) -> String {
    path.split('.').map(|s| format!("/{}", s.replace('~', "~0").replace('/', "~1"))).collect()
}

fn title_case(value: &str) -> String {
    let words = value.split(['-', '_']).filter(|w| !w.is_empty());
    let mut out = String::new();
    for (i, word) in words.enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            if i == 0 {
                out.extend(first.to_uppercase());
            } else {
                out.push(first);
            }
            out.push_str(chars.as_str());
        }
    }
    out
}

/// The section a field sits in: the longest section path that prefixes it.
fn section_of<'a>(schema: &'a SettingsSchema, field: &Field) -> Option<&'a str> {
    schema
        .sections
        .iter()
        .filter(|s| field.path.starts_with(&format!("{}.", s.path)))
        .max_by_key(|s| s.path.len())
        .map(|s| s.path.as_str())
}

fn build<T: Settings>(schema: &SettingsSchema, values: &Value, handle: Option<&SettingsHandle<T>>) -> AnyElement {
    let mut groups: Vec<(Option<&str>, Vec<&Field>)> = vec![(None, Vec::new())];
    for section in &schema.sections {
        groups.push((Some(section.path.as_str()), Vec::new()));
    }
    for field in &schema.fields {
        let at = section_of(schema, field);
        if let Some(group) = groups.iter_mut().find(|(path, _)| *path == at) {
            group.1.push(field);
        }
    }
    let sections = groups.into_iter().filter(|(_, fields)| !fields.is_empty()).map(|(path, fields)| {
        let info = path.and_then(|p| schema.sections.iter().find(|s| s.path == p));
        let mut section: FormSection = form_section();
        if let Some(info) = info {
            section = section.title(info.title.clone());
            if let Some(help) = &info.help {
                section = section.footer(help.clone());
            }
        }
        section.children(fields.into_iter().map(|f| field_row(f, values, handle)))
    });
    form().children(sections).into_any_element()
}

fn write<T: Settings>(handle: Option<&SettingsHandle<T>>, path: &str, value: Value, cx: &mut App) {
    let Some(handle) = handle else { return };
    if let Err(e) = handle.set(path, &value) {
        eprintln!("nd: settings key {path} not written: {e:?}");
    }
    cx.refresh_windows();
}

fn field_row<T: Settings>(field: &Field, values: &Value, handle: Option<&SettingsHandle<T>>) -> FormRow {
    let path = field.path.clone();
    let value = values.pointer(&pointer(&path)).filter(|v| !v.is_null()).or(field.default.as_ref()).cloned();
    let state = handle.map_or(KeyState::Editable, |h| h.key_state(&path));
    let locked = handle.is_none() || state != KeyState::Editable;
    let mut help = field.help.clone();
    let mut note = |line: String| {
        help = Some(match help.take() {
            Some(h) => format!("{h}\n{line}"),
            None => line,
        });
    };
    match &state {
        KeyState::ManagedByNix { option, .. } => note(format!("Set in your Nix configuration: {option}")),
        KeyState::ManagedByFspec { option, .. } => note(format!("Set in your fspec spec: {option}")),
        KeyState::Overridden { by } => note(format!("Overridden by {}", by.label)),
        KeyState::Editable => {}
    }
    if field.reload == Reload::Restart {
        note("Takes effect after a restart.".to_owned());
    }
    let mut row = form_row(field.title.clone()).id(format!("{path}.row"));
    if let Some(help) = help {
        row = row.help(help);
    }
    let handle = handle.cloned();
    let control: AnyElement = match &field.kind {
        FieldKind::Toggle => {
            let checked = value.as_ref().and_then(Value::as_bool).unwrap_or(false);
            let key = path.clone();
            switch(path.clone())
                .label(field.title.clone())
                .checked(checked)
                .disabled(locked)
                .on_toggle(move |e: &Toggled, _, cx| write(handle.as_ref(), &key, Value::Bool(e.checked), cx))
                .into_any_element()
        }
        FieldKind::Number { min: Some(min), max: Some(max) } => {
            let v = value.as_ref().and_then(Value::as_f64).unwrap_or(*min);
            let key = path.clone();
            let mut s = slider(path.clone(), v as f32)
                .range(*min as f32..=*max as f32)
                .label(field.title.clone())
                .disabled(locked)
                .w(px(180.));
            if let Some(step) = field.step {
                s = s.step(step as f32);
            }
            s.on_change(move |e: &SliderChanged, _, cx| {
                write(handle.as_ref(), &key, serde_json::json!(f64::from(e.value)), cx)
            })
            .into_any_element()
        }
        FieldKind::Integer { min, max } | FieldKind::Number { min, max } => {
            let integer = matches!(field.kind, FieldKind::Integer { .. });
            let v = value.as_ref().and_then(Value::as_f64).unwrap_or(0.);
            let key = path.clone();
            stepper(path.clone(), v)
                .range(min.unwrap_or(f64::from(i32::MIN))..=max.unwrap_or(f64::from(i32::MAX)))
                .step(field.step.unwrap_or(1.))
                .with_field(if integer { 0 } else { 2 })
                .label(field.title.clone())
                .disabled(locked)
                .on_change(move |e: &StepperChanged, _, cx| {
                    let value =
                        if integer { serde_json::json!(e.value.round() as i64) } else { serde_json::json!(e.value) };
                    write(handle.as_ref(), &key, value, cx)
                })
                .into_any_element()
        }
        FieldKind::Text => {
            let text = value.as_ref().and_then(Value::as_str).unwrap_or_default().to_owned();
            let key = path.clone();
            text_field(path.clone())
                .text(text)
                .label(field.title.clone())
                .disabled(locked)
                .w(px(220.))
                .on_change(move |e: &TextChanged, _, cx| {
                    write(handle.as_ref(), &key, Value::String(e.text.to_string()), cx)
                })
                .into_any_element()
        }
        FieldKind::Choice(choices) => {
            let current = value.as_ref().and_then(Value::as_str).unwrap_or_default();
            let index = choices.iter().position(|c| c.value == current).unwrap_or(0);
            let values: Vec<String> = choices.iter().map(|c| c.value.clone()).collect();
            let key = path.clone();
            select(path.clone(), choices.iter().map(|c| title_case(&c.value)))
                .selected(index)
                .label(field.title.clone())
                .disabled(locked)
                .on_change(move |e: &SelectChanged, _, cx| {
                    if let Some(v) = values.get(e.index) {
                        write(handle.as_ref(), &key, Value::String(v.clone()), cx);
                    }
                })
                .into_any_element()
        }
        FieldKind::List(_) | FieldKind::Map | FieldKind::Raw => {
            label("Edit in config.toml").secondary().id(format!("{path}.raw")).into_any_element()
        }
    };
    row.child(control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_and_pointers() {
        assert_eq!(title_case("new-note_folder"), "New note folder");
        assert_eq!(pointer("editor.font_size"), "/editor/font_size");
    }
}
