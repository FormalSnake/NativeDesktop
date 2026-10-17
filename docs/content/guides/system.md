+++
title = "System"
description = "Use dialogs, the clipboard, notifications, the keychain, a tray icon, media keys, audio, and OS events through nd::system, and add plugins for anything the framework doesn't ship."
+++

`nd::system` is how your app reaches OS services. One `System` handle fronts every service, and
the examples/system app puts each one behind a button.

<!-- docs:example system -->

## The handle

A `System` is a set of services over a `Backend`. `System::native()` talks to the OS.
`System::fake()` returns a recording backend and a handle that answers from a script, which is
what tier 1 tests use (see [testing](../tooling/testing.md)). `nd::system::system()` is the
native handle that the app already runs with, and the example's `main` passes it to `open`.

Calls that wait on the user or the OS return a `Pending<T>`, a plain oneshot future. To run one,
call `nd::system::spawn(future, continuation)`. The future runs off the UI thread, and the
continuation runs on it with `&mut App`, so it can update a view. The example sends every call
through one `run` helper that logs the outcome.

On a target that can't do something, the call fails with `Error::Unsupported`. To find out ahead
of time, call `nd::platform::has(Feature)`. That's how an app hides a button instead of showing a
failure (see [web](web.md#feature-checks)).

## Dialogs

App-level dialogs aren't tied to a window. `dialogs().open` takes `OpenOptions` (file or folder,
`multiple`, and a title) and resolves to the chosen paths, or to `None` if the user cancels.
`save` takes `SaveOptions`. `message` shows an alert and resolves to the index of the button that
the user selected.

<!-- docs:snippet examples/system/src/lib.rs#dialogs -->

An alert takes at most three buttons, the common limit across the native alerts. More than three
is `Error::Invalid`. For a sheet attached to one window, use `nd::system::window::{alert, open,
save}`, which take the window id. The example's "Sheet" button shows one.

## Clipboard

The clipboard holds text, images, and file lists. An image is
`ClipboardImage { width, height, rgba }`, with straight RGBA bytes. A read returns `None` when
the clipboard holds something else.

<!-- docs:snippet examples/system/src/lib.rs#clipboard -->

## Notifications

`notifications().show` takes a `Notification` with a title, subtitle, body, action buttons, and a
`data` value. When the user selects the notification or one of its actions, the app receives
`SystemEvent::NotificationClicked` with that `data`. This works for notifications that this
process showed.

<!-- docs:snippet examples/system/src/lib.rs#notify -->

On macOS, notifications need a bundled, signed app. An unbundled binary gets a clear error,
because macOS 27 returns no legacy notification center. Linux and Windows use `notify-rust`.

## Keychain, recent documents and the shell

`credentials()` has `set`, `get`, and `delete` over a service and account name. It uses the
Keychain on macOS, the Secret Service on Linux, and Credential Manager on Windows. A missing item
is `Error::NotFound`, and a locked keychain is `Error::Denied`.

<!-- docs:snippet examples/system/src/lib.rs#credentials -->

`recent_documents().add(path)` adds the path to the OS recent list. On Linux, the list shows only
this app's entries. `shell()` has `open_url`, `open_path`, and `reveal_path`, which are plain
synchronous calls. App directories come from `nd::system::app_dirs()` (`ensure_data`,
`ensure_cache`) and are named after the `[app]` identity in
[nativedesktop.toml](settings-and-config.md#app-manifest).

## Tray

`tray().set` creates or replaces a status item from a `Tray` builder. The builder takes an icon,
a tooltip, and menu items (plain, checkable, and separators). `TrayIcon::Named` is an SF Symbol
name on macOS and an icon theme name elsewhere. `remove` takes the item's `TrayId`. A click on the
icon arrives as `SystemEvent::Tray { item: None }`, and a menu choice carries the item id.

<!-- docs:snippet examples/system/src/lib.rs#tray -->

## Media session and audio

`audio().play(PlayOptions::path(p).volume(v).spectrum())` returns a `Player` and a future that
resolves once playback starts. `Player` has `pause`, `resume`, `stop`, `seek`, and `set_volume`.
State changes arrive as `SystemEvent::Audio` with `AudioEvent::State`. With `.spectrum()` set,
`AudioEvent::Spectrum` carries 32 log-spaced bins in 0..1, about 15 times a second. The example
draws them as bars.

`media().set(NowPlaying { .. })` publishes the title, artist, and duration to the OS controls
(Now Playing, MPRIS, and SMTC), and `clear` removes them. Media keys and the OS controls come
back as `SystemEvent::Media(MediaCommand::..)`.

<!-- docs:snippet examples/system/src/lib.rs#audio -->

## App events

Everything the OS tells the app arrives as a `SystemEvent`. That covers activation and
deactivation, URLs (`OpenUrls`), documents (`OpenFiles`), files dropped on a window (`FileDrop`,
with the window id and drop position), notification and tray clicks, media commands, and audio
state. Subscribe once with `nd::system::on_event`. Events that arrive before the first subscriber
are held and delivered to it.

<!-- docs:snippet examples/system/src/lib.rs#subscribe -->

<!-- docs:snippet examples/system/src/lib.rs#events -->

Declare URL schemes and document types in `nativedesktop.toml`.
[Packaging](packaging.md) writes them into the `Info.plist`, the `.desktop` file, and the Windows
registration:

<!-- docs:snippet examples/system/nativedesktop.toml -->

On Linux and Windows, the OS opens a second instance with the URL or path as a command-line
argument. `System::open_from_args` turns those arguments into `OpenUrls` and `OpenFiles`, and the
`nd` run does this for the launch's own arguments.

## What works where

| Service | macOS | Linux | Windows | Web |
| --- | --- | --- | --- | --- |
| Dialogs, clipboard, shell, credentials | yes | yes | code written, not run | clipboard text, `open_url` |
| Notifications | needs a bundled, signed app | yes | code written, not run | no |
| Recent documents, tray | yes | yes | code written, not run | no |
| Media session | yes | yes (MPRIS) | code written, not run | no |
| Audio and spectrum | yes (AVPlayer) | yes (GStreamer, loaded at run time) | no | no |

Linux was tested end to end on a real machine with a fake notification daemon, StatusNotifier
and MPRIS over D-Bus, an isolated keyring, and a headless Hyprland session. The Windows backend
type-checks for `x86_64-pc-windows-msvc` but hasn't run on Windows. Real media-key presses are
untested. On the [web](web.md), only clipboard text and `open_url` work.

## Extending outside the framework

For anything past this list, write a plugin. A crate implements `nd_plugin::Plugin`, the app
registers it on a `PluginHost`, and you don't fork anything in the framework. A plugin owns one
`[plugins.<name>]` table in the app's config. The host splits every `plugins` table off the
config before the app parses it, so the app keeps `deny_unknown_fields` on its own root struct.
A section that fails to parse disables that plugin without failing the app. The plugin's tasks go
through `PluginContext::spawn`, so the host can stop and restart them when the config file
changes. The app publishes any state that the plugin needs into `Services`, and the plugin
fetches it by type.

examples/discord-rpc is Discord Rich Presence written this way. It depends only on `nd-plugin`
and `nd-ipc`, the way a crate outside this repository would. Its config is a plain serde struct:

<!-- docs:snippet examples/discord-rpc/src/lib.rs#config -->

The plugin reads a `PresenceFeed` that the app published and reports a `DiscordStatus` back.
Discord often isn't running, and that's expected: the plugin keeps looking on `nd-ipc`'s
reconnect ladder and never touches the app.

<!-- docs:snippet examples/discord-rpc/src/lib.rs#plugin -->

On the app side, you register the plugin and add a config table:

<!-- docs:snippet examples/discord-rpc/src/main.rs#host -->

`nd-ipc` is the same local-socket layer that the single-instance apps use. It's covered with the
rest of the config and storage crates in [settings and config](settings-and-config.md#local-sockets).
