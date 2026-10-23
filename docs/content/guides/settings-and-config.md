+++
title = "Settings and config"
description = "The app manifest, user settings as a TOML file with layers and Nix and fspec support, machine state, local sockets, and SQLite."
+++

An app has three kinds of configuration, each in its own place. Your choices as the developer
live in `nativedesktop.toml`. The user's preferences live in a `config.toml` that the app edits
without losing the user's comments. State that the app owns (window frames, recents, a library)
lives in a `Store<T>`. Each one is a crate that you can use without the others.

<!-- docs:example notes -->

Wiring status: `nd-settings`, `settings_form`, and `Store<T>` ship and are tested. The `nd`
facade doesn't call them for you yet. `Application::settings::<T>()`, `cx.settings::<T>()`, and
`Store::load(name, cx)`, as the Notes example writes them, don't exist yet, and Notes won't build
until they do. Everything below uses the crates' real API. The Notes snippets show the settings
struct and the form, which need nothing from the facade.

## App manifest

`nativedesktop.toml` sits next to `Cargo.toml`. The name and version always come from Cargo,
never from this file. Every table denies unknown keys, so a typo is an error that names the path
and line. The examples start with a `#:schema` line that points at
`target/nd/nativedesktop.schema.json`, which `nd_config::schema_json()` produces for editors.

<!-- docs:snippet examples/notes/nativedesktop.toml -->

The tables, from [nd-config](../../../crates/nd-config/src/lib.rs):

- `[app]`: the reverse-DNS `id` (required), `display_name`, `icon`, `categories`, `url_schemes`
  (bare schemes, without `://`), and `[[app.file_associations]]` (`ext` without a dot, `role`,
  `name`, and `mime`). The schemes and associations go into `Info.plist`, the `.desktop` file,
  and the installer. The app receives them as [system events](system.md#app-events).
- `[patterns]`: project-wide pattern choices (tooltips, menus, popovers, context menus, and a
  settings window or sheet). Code can still override them per app, window, or element.
- `[webview]` and `[webview.cef]`: the engine, `system` or `cef`. Packaging bundles Chromium only
  for `cef`. See [webview](webview.md).
- `[updates]`: `feed`, `channels`, `public_key`, and `[updates.publish]`. See
  [packaging](packaging.md#releases-and-updates).
- `[diagnostics]`: `retention` overrides and `sentry_dsn_env`, the name of the variable that
  holds the DSN. See [diagnostics](../tooling/diagnostics.md).
- `[macos]`, `[windows]`, `[linux]`: for macOS, the minimum system version, signing identity,
  notarization, entitlements, and `swift_packages`. For Windows, the publisher, installer splash
  and shortcuts, MSI, and signing. For Linux, the package formats.
- `[web]`: `base_path` for the static bundle. See [web](web.md).

`validate` rejects what the schema can't express: a malformed `app.id`, a `feed` without a
`public_key`, a bad channel name, or `notarize` without a `notary_profile` and a signing identity.

## Settings struct

A settings struct is the single source of truth. The derive generates the JSON Schema, the
metadata that a settings form needs, and the typed options of the Nix module.

<!-- docs:snippet examples/notes/src/settings.rs#struct -->

`#[serde(default, deny_unknown_fields)]` is required. Defaults fill in what the file omits, and a
misspelled key fails the load. Doc comments become help text. `#[settings(range, step)]` becomes a
slider, `#[settings(reload = "restart")]` marks a key that's read once at startup, and
`#[settings(section)]` nests a struct into a titled group. Enums become pickers.

## Settings form

`settings_form::<T>()` renders the schema as native settings rows (turn on the `settings-ui`
feature on `nd`). Edits go through the handle, so the file's comments survive.

<!-- docs:snippet examples/notes/src/settings.rs#form -->

The form reads the handle from a global that you set with
`nd_components::forms::settings::provide(cx, handle)`. Without one, it shows the defaults as
read-only. A key that a Nix-managed file sets is shown read-only, with its option path and a
"Copy as Nix" action.

## Loading and editing the file

`SettingsHandle::<T>::load(Options::new("notes"))` reads every layer, lowest precedence first:

1. The defaults in the struct.
2. A Nix-managed file (any config file that resolves into `/nix/store`).
3. The user's `config.toml`.
4. Drop-ins in `config.d/*.toml`, in name order.
5. `fspec.toml`, which fspec writes from your spec (see [fspec](#fspec)).
6. `config.local.toml`, a writable overlay.
7. Environment variables named `<APP>_KEY`, where `__` nests (`NOTES_EDITOR__FONT_SIZE` sets
   `editor.font_size`).
8. Command-line overrides.

The directory is `~/Library/Application Support/<app>` on macOS, `$XDG_CONFIG_HOME/<app>` on
Linux, and `%APPDATA%\<app>` on Windows. `ND_APP_CONFIG_DIR` overrides it when set. Every key
remembers its layer: `source(key)` tells you which one, and `key_state(key)` tells you whether an
edit would stick.

Writes use `toml_edit`, so comments, ordering, and spacing survive. This test starts from a
hand-commented file:

<!-- docs:snippet crates/nd-settings/tests/gate_m7_settings.rs#commented -->

<!-- docs:snippet crates/nd-settings/tests/gate_m7_settings.rs#round-trip -->

`set` validates against the schema before it writes, so an out-of-range value or an unknown key
returns `WriteError`, and the file stays as it was. A file that no longer loads reports its path,
key, and line, and the last good settings stay in force (`Event::Rejected`). The handle watches
the files, and `on_event` delivers a `Change` with the changed keys and which of them need a
restart. The app's own writes are tagged, so the watcher ignores them.

## Nix

When the main file is a symlink into the Nix store, it's read-only. The handle then writes edits
to `config.local.toml` instead, and the managed keys refuse edits with `WriteError::ManagedByNix`.

<!-- docs:snippet crates/nd-settings/tests/gate_m7_settings.rs#read-only -->

The home-manager module is generated, not written by hand. `nd-hm-gen --schema
settings.schema.json --out nix/settings-options.nix` turns the committed schema into typed options
for `programs.<app>.settings`, and `--check` fails when the file is out of date. `mkApp` in
[nix/mk-app.nix](../../../nix/mk-app.nix) builds the package with `crane` and wires up the module
([nix/hm-module.nix](../../../nix/hm-module.nix)): `settings` written to `config.toml`, `theme`
tokens, `autostart`, and `environmentFile`. Its checks include clippy, nextest, the module eval,
and a check that the settings schema is in sync. Updates are off for a binary under `/nix/store`.

## fspec

On a machine that [fspec](https://github.com/FormalStudio/FormalSpec) manages, the app installs
and updates through fspec instead of Nix. Each app ships a generated `fspec.star` at its repo
root, and a spec loads it straight from the app's repo:

```python
load("gh:FormalStudio/formalmusic@main//fspec.star", "formalmusic")

hosts = [host("e1504g", platform = "x86_64-linux", system = tumbleweed(), modules = [
    formalmusic(settings = {"playback": {"quality": "high"}}, autostart = True),
])]
```

`@main` tracks the branch, so `fspec update` moves the lock to its newest commit. `@<commit>`
pins it. The package builds the same commit its `fspec.star` came from: `fspec build` compiles it
on Linux in a Tumbleweed container, with Zig for the terminal widget and `ND_NO_UPDATES=1`. `rev`
builds another commit, and `source` takes any fspec source instead (macOS needs one, since there's
no build recipe for it yet). The other arguments match the home-manager module: `settings` is
written to `fspec.toml`, `theme` to `theme.json`, `autostart` adds a user service tied to the
graphical session, and `environment_file` passes secrets to it. Removing the module removes the
app, its files and its unit, and moves the config directory to fspec's trash.

`settings` is checked when the spec is evaluated. An unknown key, a value of the wrong type, a
choice that isn't listed, or a number out of `range` fails the plan with the key's path. fspec
copies `fspec.toml` instead of linking it, so the layer sits above the user's files, and the spec
wins. Its keys are read-only, with `WriteError::ManagedByFspec` and a "Copy as fspec" literal.
Every other key stays editable in `config.toml`.

<!-- docs:snippet crates/nd-settings/tests/schema_fspec.rs#fspec -->

`fspec.star` is generated from the same schema as the Nix options, so the two can't drift.
`nd_settings::fspec::star_module(&App::new(name, repo), &schema)` returns the file. An app checks
it in the test that already checks `nix/settings-options.nix`. `nd-fspec-gen --schema
settings.schema.json --name <app> --repo <url> --out fspec.star` writes it from a committed
schema, and `--check` fails when the file is out of date. `App::build_deps` adds openSUSE packages
that the app's own crates need on top of nd's.

## Machine state

`Store<T>` holds state that isn't a preference. On disk, it's JSON in the form `{"version": N,
"data": ..}`. Writes are debounced (250 ms, and never more than 2 s behind), go through a temp
file and a rename, and flush on drop. If a file can't be read or migrated, the store moves it
aside as `<name>.corrupt-<unix time>`, the defaults win, and `load_error()` says why. A `Migrate`
function upgrades older versions on load.

<!-- docs:snippet crates/nd-settings/tests/store.rs#store -->

## Web

On the web, every settings file and every `Store<T>` is a `localStorage` key, with the same API.
There's no file watcher and no Nix layer. See [web](web.md).

## Local sockets

`nd-ipc` is JSON over a Unix socket or a Windows named pipe, in three layers on one `Endpoint`.

`instance::acquire` is the single-instance lock. The primary instance holds a file lock, which
the OS releases when the process dies, so a crash leaves no stale lock. A second launch forwards
its arguments and URLs to the primary, gets `Instance::Forwarded`, and then exits.

<!-- docs:snippet crates/nd-ipc/tests/gate_m7_ipc.rs#instance -->

`rpc` is JSON-RPC 2.0 with typed contracts. A `Method` fixes the name, parameters, and result, so
the `Server` handler and every `Client` call agree at compile time. The client reconnects on a
`Ladder` and fails calls fast while it's disconnected.

<!-- docs:snippet crates/nd-ipc/tests/gate_m7_ipc.rs#method -->

`connect` and `connect_path` give you a raw stream for protocols that aren't JSON lines. The
[Discord plugin](system.md#extending-outside-the-framework) uses it.

## SQLite

`nd-data` is one SQLite connection on its own thread behind an async API. Its futures are plain
oneshot receivers, so they run under GPUI's `cx.spawn`, tokio, or `block_on`. It doesn't depend
on an ORM: `SqliteExecutor` is the contract that an adapter targets. `migrate` applies numbered
migrations once and returns the schema version.

<!-- docs:snippet crates/nd-data/tests/gate_m7_data.rs#migrations -->

<!-- docs:snippet crates/nd-data/tests/gate_m7_data.rs#query -->

`transaction` is all or nothing: if one statement fails, the whole batch rolls back. `nd` doesn't
re-export `nd-data`, so add the crate directly. It bundles SQLite through `rusqlite` and isn't
built for the web.
