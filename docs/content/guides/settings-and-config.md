+++
title = "Settings and config"
description = "The app manifest, user settings as a TOML file with layers and Nix support, machine state, local sockets and SQLite."
+++

Three kinds of configuration, in three places. The developer's choices about the app live in
`nativedesktop.toml`. The user's preferences live in a `config.toml` that the app edits without
losing the user's comments. State the app owns (window frames, recents, a library) lives in a
`Store<T>`. Each is a crate you can use without the others.

<!-- docs:example notes -->

Wiring status: `nd-settings`, `settings_form` and `Store<T>` ship and are tested. The `nd`
facade does not call them for you yet, so `Application::settings::<T>()`, `cx.settings::<T>()` and
`Store::load(name, cx)` as the Notes example writes them do not exist, and Notes does not build
until they do. Everything below names the crates' real API; the Notes snippets show the settings
struct and the form, which need nothing from the facade.

## App manifest

`nativedesktop.toml` sits beside `Cargo.toml`. Name and version come from Cargo, never from this
file. Every table denies unknown keys, so a typo is an error with the path and line. The
examples start with a `#:schema` line pointing at `target/nd/nativedesktop.schema.json`, which
`nd_config::schema_json()` produces for editors.

<!-- docs:snippet examples/notes/nativedesktop.toml -->

The tables, from [nd-config](../../../crates/nd-config/src/lib.rs):

- `[app]`: reverse-DNS `id` (required), `display_name`, `icon`, `categories`, `url_schemes`
  (bare schemes, no `://`) and `[[app.file_associations]]` (`ext` without a dot, `role`, `name`,
  `mime`). The schemes and associations reach `Info.plist`, the `.desktop` file and the
  installer; the app receives them as [system events](system.md#app-events).
- `[patterns]`: project-wide pattern choices (tooltips, menus, popovers, context menus, settings
  window or sheet) that code can still override per app, window or element.
- `[webview]` and `[webview.cef]`: the engine, `system` or `cef`. Packaging bundles Chromium only
  for `cef`. See [webview](webview.md).
- `[updates]`: `feed`, `channels`, `public_key` and `[updates.publish]`. See
  [packaging](packaging.md#releases-and-updates).
- `[diagnostics]`: `retention` overrides and `sentry_dsn_env`, the name of the variable that
  holds the DSN. See [diagnostics](../tooling/diagnostics.md).
- `[macos]`, `[windows]`, `[linux]`: minimum system version, signing identity, notarization,
  entitlements and `swift_packages`; publisher, installer splash and shortcuts, MSI, signing;
  package formats.
- `[web]`: `base_path` for the static bundle. See [web](web.md).

`validate` rejects what the schema cannot express: a malformed `app.id`, a `feed` without a
`public_key`, a bad channel name, `notarize` without a `notary_profile` and a signing identity.

## Settings struct

A settings struct is the single source. The derive gives the JSON Schema, the metadata a settings
form needs and the typed options of the Nix module.

<!-- docs:snippet examples/notes/src/settings.rs#struct -->

`#[serde(default, deny_unknown_fields)]` is required: defaults fill what the file omits, and a
misspelled key fails the load. Doc comments become help text. `#[settings(range, step)]` becomes a
slider, `#[settings(reload = "restart")]` marks a key read once at startup, and
`#[settings(section)]` nests a struct into a titled group. Enums become pickers.

## Settings form

`settings_form::<T>()` renders the schema as native settings rows (feature `settings-ui` on `nd`).
Edits go through the handle, so the file's comments survive.

<!-- docs:snippet examples/notes/src/settings.rs#form -->

The form reads the handle from a global set with
`nd_components::forms::settings::provide(cx, handle)`. Without one it shows the defaults,
read-only. A key that a Nix-managed file sets shows read-only with its option path and a "Copy as
Nix" action.

## Loading and editing the file

`SettingsHandle::<T>::load(Options::new("notes"))` reads every layer, lowest precedence first:

1. the defaults in the struct,
2. a Nix-managed file (any config file that resolves into `/nix/store`),
3. the user's `config.toml`,
4. drop-ins in `config.d/*.toml`, in name order,
5. `config.local.toml`, a writable overlay,
6. environment variables, `<APP>_KEY`, where `__` nests (`NOTES_EDITOR__FONT_SIZE` sets
   `editor.font_size`),
7. command-line overrides.

The directory is `~/Library/Application Support/<app>` on macOS, `$XDG_CONFIG_HOME/<app>` on
Linux, `%APPDATA%\<app>` on Windows, or `ND_APP_CONFIG_DIR` when set. Every key remembers its
layer: `source(key)` says which, and `key_state(key)` says whether an edit would stick.

Writes use `toml_edit`, so comments, ordering and spacing survive. This test starts from a
hand-commented file:

<!-- docs:snippet crates/nd-settings/tests/gate_m7_settings.rs#commented -->

<!-- docs:snippet crates/nd-settings/tests/gate_m7_settings.rs#round-trip -->

`set` validates against the schema before it writes, so an out-of-range value or an unknown key
returns `WriteError` and the file stays as it was. A file that no longer loads reports its path,
key and line, and the last good settings stay in force (`Event::Rejected`). The handle watches
the files; `on_event` delivers a `Change` with the changed keys and which of them need a restart.
The app's own writes are tagged so the watcher ignores them.

## Nix

When the main file is a symlink into the Nix store, it is read-only. The handle then writes edits
to `config.local.toml` instead, and the managed keys refuse edits with `WriteError::ManagedByNix`.

<!-- docs:snippet crates/nd-settings/tests/gate_m7_settings.rs#read-only -->

The home-manager module is generated, not written by hand. `nd-hm-gen --schema
settings.schema.json --out nix/settings-options.nix` turns the committed schema into typed options
for `programs.<app>.settings`, and `--check` fails when the file is stale. `mkApp` in
[nix/mk-app.nix](../../../nix/mk-app.nix) builds the package with `crane` and wires the module
([nix/hm-module.nix](../../../nix/hm-module.nix)): `settings` written to `config.toml`, `theme`
tokens, `autostart` and `environmentFile`. Its checks include clippy, nextest, the module eval
and that the settings schema is in sync. Updates are off for a binary under `/nix/store`.

## Machine state

`Store<T>` holds state that is not a preference. It is JSON on disk as `{"version": N, "data":
..}`. Writes are debounced (250 ms, never more than 2 s behind), go through a temp file and a
rename, and flush on drop. A file that cannot be read or migrated is moved aside as
`<name>.corrupt-<unix time>`, the defaults win, and `load_error()` says why. A `Migrate`
function upgrades older versions on load.

<!-- docs:snippet crates/nd-settings/tests/store.rs#store -->

## Web

On the web every settings file and every `Store<T>` is a `localStorage` key, with the same API.
There is no file watcher and no Nix layer. See [web](web.md).

## Local sockets

`nd-ipc` is JSON over a unix socket or a Windows named pipe, in three layers on one `Endpoint`.

`instance::acquire` is the single-instance lock. The primary instance holds a file lock, which
the OS releases when the process dies, so a crash leaves no stale lock. A second launch forwards
its arguments and URLs to the primary and gets `Instance::Forwarded`; then it exits.

<!-- docs:snippet crates/nd-ipc/tests/gate_m7_ipc.rs#instance -->

`rpc` is JSON-RPC 2.0 with typed contracts: a `Method` fixes the name, parameters and result, so
the `Server` handler and every `Client` call agree at compile time. The client reconnects on a
`Ladder` and fails calls fast while it is disconnected.

<!-- docs:snippet crates/nd-ipc/tests/gate_m7_ipc.rs#method -->

`connect` and `connect_path` give a raw stream for protocols that are not JSON lines; the
[Discord plugin](system.md#extending-outside-the-framework) uses it.

## SQLite

`nd-data` is one SQLite connection on its own thread behind an async API. Its futures are plain
oneshot receivers, so they run under GPUI's `cx.spawn`, tokio or `block_on`. It depends on no
ORM: `SqliteExecutor` is the contract an adapter targets. `migrate` applies numbered migrations
once and returns the schema version.

<!-- docs:snippet crates/nd-data/tests/gate_m7_data.rs#migrations -->

<!-- docs:snippet crates/nd-data/tests/gate_m7_data.rs#query -->

`transaction` is all or nothing: a failing statement rolls the whole batch back. `nd-data` is not
re-exported by `nd`; add the crate directly. It bundles SQLite through `rusqlite` and is not built
for the web.
