// The Nix-managed test is unix only; its imports go unused on Windows.
#![cfg_attr(windows, allow(unused_imports))]
mod common;

use common::{Dirs, NotesSettings};
use nd_plugin::{Plugin, PluginContext, PluginError, PluginHost, Services};
use nd_settings::{FieldKind, KeyState, SettingsHandle, nix, schema_json};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct DiscordConfig {
    enabled: bool,
    application_id: String,
    buttons: Vec<String>,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self { enabled: true, application_id: String::new(), buttons: Vec::new() }
    }
}

struct Discord;

impl Plugin for Discord {
    type Config = DiscordConfig;
    fn name(&self) -> &'static str {
        "discord"
    }
    fn description(&self) -> &'static str {
        "Show what you are doing as Discord rich presence."
    }
    fn init(&mut self, _: &PluginContext, _: DiscordConfig) -> Result<(), PluginError> {
        Ok(())
    }
}

fn host() -> (tokio::runtime::Runtime, PluginHost) {
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let mut host = PluginHost::new("notes", rt.handle().clone(), Services::new());
    host.register(Discord);
    (rt, host)
}

#[test]
fn plugin_sections_layer_over_plugin_defaults_and_feed_the_host() {
    let (_rt, mut host) = host();
    let dirs = Dirs::new();
    dirs.write("config.toml", "compact_rows = true\n[plugins.discord]\napplication_id = \"123\"\n");
    let opts = dirs.options("notes").plugin_defaults(host.default_sections());
    let settings = SettingsHandle::<NotesSettings>::load(opts).unwrap();

    let section = settings.plugin_section("discord").unwrap();
    assert_eq!(section["application_id"].as_str(), Some("123"));
    assert_eq!(section["enabled"].as_bool(), Some(true), "plugin default fills the gap");

    let mut root = settings.plugins_root();
    let report = host.load(&mut root);
    assert_eq!(report.started, ["discord"]);
    assert!(report.failed.is_empty());

    // The app's strict struct still parses with the plugin section present.
    assert!(settings.get().compact_rows);
}

// Nix-managed means a symlink into the store, which only exists on unix.
#[cfg(unix)]
#[test]
fn plugin_keys_are_typed_form_fields_and_nix_managed_keys_are_read_only() {
    let (_rt, host) = host();
    let dirs = Dirs::new();
    let managed = dirs.store.join("cfg.toml");
    std::fs::write(&managed, "[plugins.discord]\napplication_id = \"nix\"\n").unwrap();
    std::os::unix::fs::symlink(&managed, dirs.user()).unwrap();
    let settings =
        SettingsHandle::<NotesSettings>::load(dirs.options("notes").plugin_defaults(host.default_sections())).unwrap();

    let field = settings.schema().field("plugins.discord.enabled").unwrap();
    assert_eq!(field.kind, FieldKind::Toggle);
    assert_eq!(
        settings.schema().field("plugins.discord.buttons").unwrap().kind,
        FieldKind::List(Box::new(FieldKind::Raw))
    );
    assert!(matches!(
        settings.key_state("plugins.discord.application_id"),
        KeyState::ManagedByNix { option, .. } if option == "programs.notes.settings.plugins.discord.application_id"
    ));
    settings.set("plugins.discord.enabled", &false).unwrap();
    assert_eq!(settings.plugin_section("discord").unwrap()["enabled"].as_bool(), Some(false));
}

#[test]
fn generated_module_types_plugin_sections() {
    let (_rt, host) = host();
    let module = nix::options_module_with_plugins(
        &schema_json::<NotesSettings>(),
        &host.default_sections(),
        &host.descriptions(),
    );
    for expected in [
        "plugins = lib.mkOption",
        "discord = lib.mkOption",
        "enabled = opt t.bool",
        "application_id = opt t.str",
        "buttons = opt (t.listOf tomlType)",
        "freeformType = t.attrsOf tomlType;",
        "Show what you are doing as Discord rich presence.",
    ] {
        assert!(module.contains(expected), "missing `{expected}` in:\n{module}");
    }
}

#[test]
fn committed_plugin_options_match() {
    let (_rt, host) = host();
    let module = nix::options_module_with_plugins(
        &schema_json::<NotesSettings>(),
        &host.default_sections(),
        &host.descriptions(),
    );
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notes-plugins/nix/settings-options.nix");
    if std::env::var_os("ND_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &module).unwrap();
    }
    assert!(
        std::fs::read_to_string(&path).unwrap_or_default() == module,
        "{} is stale, run with ND_BLESS=1",
        path.display()
    );
}
