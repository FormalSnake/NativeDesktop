+++
title = "System"
description = "Dialogs, clipboard, notifications, keychain, tray, media keys, audio and OS events through nd::system, plus plugins for what the framework does not ship."
+++

`nd::system` is the app's door to OS services. One `System` handle fronts every service, and the
examples/system app puts each of them behind a button.

<!-- docs:example system -->

## The handle

A `System` is a set of services over a `Backend`. `System::native()` talks to the OS,
`System::fake()` returns a recording backend and a handle that answers from a script, which is
what tier 1 tests use (see [testing](../tooling/testing.md)). `nd::system::system()` is the
native handle the app already runs with; the example's `main` passes it to `open`.

Calls that wait on the user or the OS return a `Pending<T>`, a plain oneshot future. Run one
with `nd::system::spawn(future, continuation)`: the future runs off the UI thread and the
continuation runs on it with `&mut App`, so it can update a view. The example funnels every
call through one `run` helper that logs the outcome.

On a target that cannot do something, the call fails with `Error::Unsupported`.
`nd::platform::has(Feature)` answers ahead of time, which is how an app hides a button instead of
showing a failure (see [web](web.md#feature-checks)).

## Dialogs

App-level dialogs are not tied to a window. `dialogs().open` takes `OpenOptions` (file or folder,
`multiple`, a title) and resolves to the chosen paths or `None` on cancel. `save` takes
`SaveOptions`. `message` shows an alert and resolves to the index of the pressed button.

<!-- docs:snippet examples/system/src/lib.rs#dialogs -->

An alert takes at most three buttons, the common limit of the native alerts; more is
`Error::Invalid`. A sheet attached to one window comes from `nd::system::window::{alert, open,
save}`, which take the window id; the example's "Sheet" button shows it.

## Clipboard

Text, images and file lists. An image is `ClipboardImage { width, height, rgba }`, straight RGBA
bytes. Reading returns `None` when the clipboard holds something else.

<!-- docs:snippet examples/system/src/lib.rs#clipboard -->

## Notifications

`notifications().show` takes a `Notification` with a title, subtitle, body, action buttons and a
`data` value. Clicking the notification or an action arrives as `SystemEvent::NotificationClicked`
with that `data` back, for notifications this process showed.

<!-- docs:snippet examples/system/src/lib.rs#notify -->

On macOS notifications need a bundled, signed app. An unbundled binary gets a clear error,
because macOS 27 returns no legacy notification center. Linux and Windows use `notify-rust`.

## Keychain, recent documents and the shell

`credentials()` has `set`, `get` and `delete` over a service and account name: the Keychain on
macOS, the Secret Service on Linux, Credential Manager on Windows. A missing item is
`Error::NotFound`; a locked keychain is `Error::Denied`.

<!-- docs:snippet examples/system/src/lib.rs#credentials -->

`recent_documents().add(path)` feeds the OS recent list (the entries of this app only on Linux).
`shell()` has `open_url`, `open_path` and `reveal_path`, and they are plain synchronous calls.
App directories come from `nd::system::app_dirs()` (`ensure_data`, `ensure_cache`), named by
the `[app]` identity in [nativedesktop.toml](settings-and-config.md#app-manifest).

## Tray

`tray().set` creates or replaces a status item from a `Tray` builder: an icon (`TrayIcon::Named`
is an SF Symbol name on macOS and an icon theme name elsewhere), a tooltip and menu items
(plain, checkable, separators). `remove` takes its `TrayId`. A click on the icon is
`SystemEvent::Tray { item: None }`, a menu choice carries the item id.

<!-- docs:snippet examples/system/src/lib.rs#tray -->

## Media session and audio

`audio().play(PlayOptions::path(p).volume(v).spectrum())` returns a `Player` and a future that
resolves once playback started. `Player` has `pause`, `resume`, `stop`, `seek` and `set_volume`.
State changes arrive as `SystemEvent::Audio` with `AudioEvent::State`, and with `.spectrum()`
set, `AudioEvent::Spectrum` carries 32 log-spaced bins in 0..1, about 15 times a second. The
example draws them as bars.

`media().set(NowPlaying { .. })` publishes the title, artist and duration to the OS controls
(Now Playing, MPRIS, SMTC); `clear` removes it. Media keys and the controls come back as
`SystemEvent::Media(MediaCommand::..)`.

<!-- docs:snippet examples/system/src/lib.rs#audio -->

## App events

Everything the OS tells the app arrives as a `SystemEvent`: activation and deactivation, URLs
(`OpenUrls`), documents (`OpenFiles`), files dropped on a window (`FileDrop`, with the window id
and drop position), notification and tray clicks, media commands and audio state. Subscribe
once with `nd::system::on_event`; events that arrive before the first subscriber are held and
delivered to it.

<!-- docs:snippet examples/system/src/lib.rs#subscribe -->

<!-- docs:snippet examples/system/src/lib.rs#events -->

URL schemes and document types are declared in `nativedesktop.toml`, and
[packaging](packaging.md) writes them into the `Info.plist`, the `.desktop` file and the Windows
registration:

<!-- docs:snippet examples/system/nativedesktop.toml -->

On Linux and Windows the OS opens a second instance with the URL or path as a command-line
argument. `System::open_from_args` turns those arguments into `OpenUrls` and `OpenFiles`, and
the `nd` run does it for the launch's own arguments.

## What works where

| Service | macOS | Linux | Windows | Web |
| --- | --- | --- | --- | --- |
| Dialogs, clipboard, shell, credentials | yes | yes | code written, not run | clipboard text, `open_url` |
| Notifications | needs a bundled, signed app | yes | code written, not run | no |
| Recent documents, tray | yes | yes | code written, not run | no |
| Media session | yes | yes (MPRIS) | code written, not run | no |
| Audio and spectrum | yes (AVPlayer) | yes (GStreamer, loaded at run time) | no | no |

Linux was driven end to end on a real machine with a fake notification daemon, StatusNotifier
and MPRIS over D-Bus, an isolated keyring, and a headless Hyprland session. The Windows backend
type-checks for `x86_64-pc-windows-msvc` and has not run on Windows. Real media-key presses are
untested. [Web](web.md) has clipboard text and `open_url` only.

## Extending outside the framework

Anything past this list is a plugin: a crate implements `nd_plugin::Plugin`, the app registers it
on a `PluginHost`, and nothing in the framework is forked. A plugin owns one `[plugins.<name>]`
table in the app's config. The host splits every `plugins` table off the config before the app
parses it, so the app keeps `deny_unknown_fields` on its own root struct, and a section that
fails to parse disables that plugin without failing the app. The plugin's tasks go through
`PluginContext::spawn`, so the host can stop and restart them when the config file changes.
State the plugin needs from the app is published into `Services` and fetched by type.

examples/discord-rpc is Discord Rich Presence written that way. It depends on `nd-plugin` and
`nd-ipc` only, the way a crate outside this repository would. Its config is a plain serde
struct:

<!-- docs:snippet examples/discord-rpc/src/lib.rs#config -->

The plugin reads a `PresenceFeed` the app published, and reports a `DiscordStatus` back. Discord
not running is the normal case: the plugin keeps looking on `nd-ipc`'s reconnect ladder and
never touches the app.

<!-- docs:snippet examples/discord-rpc/src/lib.rs#plugin -->

The app side is a registration and a config table:

<!-- docs:snippet examples/discord-rpc/src/main.rs#host -->

`nd-ipc` is the same local-socket layer the single-instance apps use; it is covered with the
rest of the config and storage crates in [settings and config](settings-and-config.md#local-sockets).
