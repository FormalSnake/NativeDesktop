//! Tier 1 behaviour of `settings_form::<T>()`.

#![cfg(feature = "settings")]

mod common;

use std::fs;

use common::*;
use nd_components::form::FormSectionProps;
use nd_components::forms::settings::{provide, settings_form};
use nd_components::forms::*;
use nd_settings::{Options, Paths, Settings, SettingsHandle};
use nd_ui::Role;
use nd_ui::prelude::*;
use nd_ui::semantics::SemanticAction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

#[derive(Settings, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct Prefs {
    /// Show files that start with a dot.
    show_hidden: bool,
    #[settings(range = 11.0..=24.0, step = 1.0)]
    font_size: f32,
    #[settings(range = 1..=8)]
    columns: u32,
    sort: SortOrder,
    name: String,
    #[settings(section)]
    editor: Editor,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
enum SortOrder {
    Name,
    Date,
    Size,
}

#[derive(Settings, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct Editor {
    wrap: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            show_hidden: false,
            font_size: 15.,
            columns: 3,
            sort: SortOrder::Name,
            name: "Ada".into(),
            editor: Editor::default(),
        }
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self { wrap: true }
    }
}

fn view<T>(_: &T, _: &mut Context<Host<T>>) -> AnyElement {
    v_flex().child(settings_form::<Prefs>()).into_any_element()
}

struct Loaded {
    dir: TempDir,
    handle: SettingsHandle<Prefs>,
}

impl Loaded {
    fn config(&self) -> std::path::PathBuf {
        self.dir.path().join("config").join("config.toml")
    }
}

fn options(dir: &TempDir) -> Options {
    let config = dir.path().join("config");
    let store = dir.path().join("store");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&store).unwrap();
    Options::new("prefs").paths(Paths::in_dir(&config)).env(Vec::new()).store_roots(vec![store])
}

fn loaded() -> Rig<Loaded> {
    Rig::build(
        |cx| {
            let dir = tempfile::tempdir().unwrap();
            let handle = SettingsHandle::<Prefs>::load(options(&dir)).unwrap();
            provide(cx, handle.clone());
            Loaded { dir, handle }
        },
        view,
    )
}

#[test]
fn without_a_handle_the_defaults_show_read_only() {
    let rig = Rig::new((), view);
    for id in ["show_hidden", "font_size", "columns", "sort", "name", "editor.wrap"] {
        assert!(!rig.sem(id).enabled, "{id} is disabled");
    }
    assert_eq!(rig.sem("show_hidden").checked, Some(false));
    assert_eq!(rig.sem("font_size").value.as_deref(), Some("15"));
    assert_eq!(rig.sem("name").value.as_deref(), Some("Ada"));
    assert_eq!(rig.sem("editor.wrap").checked, Some(true));
}

#[test]
fn field_kinds_map_to_controls_with_their_bounds() {
    let rig = Rig::new((), view);
    assert!(rig.kind_is::<SwitchKind>("show_hidden"));
    assert_eq!(rig.sem("show_hidden").name, "Show hidden");

    assert!(rig.kind_is::<SliderKind>("font_size"));
    let slider = rig.props::<SliderProps>("font_size");
    assert_eq!((slider.min, slider.max, slider.step), (11., 24., Some(1.)));

    assert!(rig.kind_is::<StepperKind>("columns"));
    let stepper = rig.props::<StepperProps>("columns");
    assert_eq!((stepper.min, stepper.max, stepper.field), (1., 8., Some(0)));
    assert_eq!(rig.sem("columns").value.as_deref(), Some("3"));

    assert!(rig.kind_is::<SelectKind>("sort"));
    let select = rig.props::<SelectProps>("sort");
    let options: Vec<_> = select.options.iter().map(|o| o.as_ref()).collect();
    assert_eq!((options, select.selected), (vec!["Name", "Date", "Size"], 0));
    assert_eq!(rig.sem("sort").role, Role::PopUpButton);

    assert!(rig.kind_is::<TextFieldKind>("name"));
    assert_eq!(rig.sem("name").role, Role::TextField);
}

#[test]
fn sections_become_form_sections_and_doc_comments_become_help() {
    let rig = Rig::new((), view);
    let titles: Vec<_> = rig
        .find_props::<FormSectionProps>()
        .into_iter()
        .map(|n| rig.cx.backend().props(n).unwrap().downcast_ref::<FormSectionProps>().unwrap().title.clone())
        .map(|t| t.map(|t| t.to_string()))
        .collect();
    assert_eq!(titles, [None, Some("Editor".to_owned())]);
    let row = rig.cx.backend().props(rig.node("show_hidden.row")).unwrap();
    let row = row.downcast_ref::<nd_components::form::FormRowProps>().unwrap();
    assert_eq!((row.label.as_ref(), row.help.as_deref()), ("Show hidden", Some("Show files that start with a dot.")));
    let wrap_section = rig.cx.backend().parent(rig.node("editor.wrap.row")).unwrap();
    assert!(rig.cx.backend().props(wrap_section).unwrap().downcast_ref::<FormSectionProps>().is_some());
}

#[test]
fn a_provided_handle_makes_the_controls_editable() {
    let rig = loaded();
    for id in ["show_hidden", "font_size", "columns", "sort", "name", "editor.wrap"] {
        assert!(rig.sem(id).enabled, "{id} is enabled");
    }
}

#[test]
fn toggling_a_switch_writes_the_file_and_the_next_render_shows_it() {
    let mut rig = loaded();
    assert!(!rig.state().config().exists());
    rig.perform("show_hidden", SemanticAction::Click).unwrap();
    assert!(rig.state().handle.get().show_hidden);
    let file = fs::read_to_string(rig.state().config()).unwrap();
    assert!(file.contains("show_hidden = true"), "{file}");
    assert_eq!(rig.sem("show_hidden").checked, Some(true));

    rig.perform("editor.wrap", SemanticAction::Click).unwrap();
    let file = fs::read_to_string(rig.state().config()).unwrap();
    assert!(file.contains("[editor]") && file.contains("wrap = false"), "{file}");
    assert_eq!(rig.sem("editor.wrap").checked, Some(false));
}

#[test]
fn slider_stepper_select_and_text_edits_reach_the_file() {
    let mut rig = loaded();
    rig.set_value("font_size", "40").unwrap();
    assert_eq!(rig.state().handle.get().font_size, 24., "clamped to the declared range");
    assert_eq!(rig.sem("font_size").value.as_deref(), Some("24"));

    rig.set_value("columns", "6").unwrap();
    assert_eq!(rig.state().handle.get().columns, 6);
    assert_eq!(rig.sem("columns").value.as_deref(), Some("6"));

    rig.set_value("sort", "Size").unwrap();
    assert_eq!(rig.state().handle.get().sort, SortOrder::Size);
    assert_eq!(rig.sem("sort").value.as_deref(), Some("Size"));

    rig.set_value("name", "Grace").unwrap();
    assert_eq!(rig.state().handle.get().name, "Grace");
    assert_eq!(rig.sem("name").value.as_deref(), Some("Grace"));

    let file = fs::read_to_string(rig.state().config()).unwrap();
    for line in ["font_size = 24.0", "columns = 6", "sort = \"size\"", "name = \"Grace\""] {
        assert!(file.contains(line), "{line} in\n{file}");
    }
}

#[test]
fn a_value_the_file_holds_shows_on_first_render() {
    let mut rig = Rig::build(
        |cx| {
            let dir = tempfile::tempdir().unwrap();
            let options = options(&dir);
            fs::write(dir.path().join("config").join("config.toml"), "columns = 5\nsort = \"date\"\n").unwrap();
            let handle = SettingsHandle::<Prefs>::load(options).unwrap();
            provide(cx, handle.clone());
            Loaded { dir, handle }
        },
        view,
    );
    assert_eq!(rig.sem("columns").value.as_deref(), Some("5"));
    assert_eq!(rig.sem("sort").value.as_deref(), Some("Date"));
    rig.fresh();
    rig.rerender();
    assert_eq!(rig.all_updates(), 0, "rendering again changes nothing");
}

#[cfg(unix)]
#[test]
fn a_key_managed_by_nix_shows_disabled_with_its_option_path() {
    let mut rig = Rig::build(
        |cx| {
            let dir = tempfile::tempdir().unwrap();
            let options = options(&dir);
            let managed = dir.path().join("store").join("abc-prefs-config.toml");
            fs::write(&managed, "columns = 7\n").unwrap();
            std::os::unix::fs::symlink(&managed, dir.path().join("config").join("config.toml")).unwrap();
            let handle = SettingsHandle::<Prefs>::load(options).unwrap();
            provide(cx, handle.clone());
            Loaded { dir, handle }
        },
        view,
    );
    assert!(!rig.sem("columns").enabled);
    assert_eq!(rig.sem("columns").value.as_deref(), Some("7"));
    let row = rig.cx.backend().props(rig.node("columns.row")).unwrap();
    let help = row.downcast_ref::<nd_components::form::FormRowProps>().unwrap().help.clone().unwrap();
    assert!(help.contains("Set in your Nix configuration: programs.prefs.settings.columns"), "{help}");
    assert!(refused(rig.set_value("columns", "2")));
    assert!(rig.sem("name").enabled, "keys the managed file leaves alone stay editable");
}
