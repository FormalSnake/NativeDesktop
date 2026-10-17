+++
title = "Patterns"
description = "Declare window structure such as the toolbar, sidebar, inspector, and titlebar once, and let each OS draw it."
+++

Declare the structure of a window with intents (`scaffold`, `toolbar`, `sidebar`, `inspector`,
`titlebar`), and let the OS draw it. On macOS, each intent is the system's own component, so a new
macOS release restyles your app without any change from you. Don't rebuild a sidebar or toolbar
from `div`s, because a pixel copy goes stale as soon as the OS changes. The Chrome example declares
a Mail-style window and names no AppKit class.

<!-- docs:example chrome -->

## What each intent becomes

| Intent | macOS (AppKit) | Linux and Windows (drawn through GPUI) |
| --- | --- | --- |
| `scaffold().sidebar()` | sidebar split item hosting a source list | pane on the preset's sidebar surface |
| `scaffold().inspector()` | inspector split item | right pane |
| `toolbar()` | `NSToolbar` with system items and native item views | header row |
| `titlebar()` | content under a transparent titlebar, inset for the traffic lights | inset for the caption buttons or the compositor's `button-layout` |
| `window_controls()` | the real traffic lights | caption buttons only under client-side decorations |
| `.context_menu()` | `NSMenu` | menu drawn in the overlay stack |
| `Alert` | `NSAlert`, a sheet on the window | dialog with a scrim |
| menu bar (`set_menus`) | global `NSMenu` | no bar: the items are folded into the command palette (the web draws a bar above its canvas) |
| settings window | toolbar-tab window with grouped rows, `cmd-,` | separate window |
| `glass()` | `NSGlassEffectView` on macOS 26 and later, `NSVisualEffectView` before | the preset's material: Acrylic on Windows, a transparent surface the compositor may blur on Linux |

On Windows, the drawn chrome follows WinUI and is checked against Windows 11's own apps, such as
Settings and Task Manager:

- The sidebar and toolbar are one Mica surface with no divider.
- The content sits on an opaque layer whose leading corner takes the window's radius.
- The sidebar draws like NavigationView, with Segoe Fluent icons and a subtle fill with an accent
  pill on the selected row. Its toggle folds it to a 48 px icon rail.
- The caption buttons are as tall as the toolbar, and the window title uses the 12 pt caption
  style.
- `banner` is an inset card like InfoBar.

Form controls, dialogs, and menus still use the Adwaita look on Windows. Linux is checked on
Hyprland.

## Scaffold, toolbar, sidebar and inspector

`scaffold()` takes a toolbar, a sidebar, the content, and an inspector. Every part is optional.
The toolbar holds system items (`sidebar_toggle`, `inspector_toggle`, `flexible_space`) next to
your own buttons, and macOS gives them the system look and sizing.

<!-- docs:snippet examples/chrome/src/lib.rs#toolbar -->

The sidebar holds a `source_list`: sections of items, each with an icon and a badge. Your state
owns the selection. On macOS, the list is an `NSOutlineView` in source-list style.

<!-- docs:snippet examples/chrome/src/lib.rs#sidebar -->

Your state controls whether the inspector is collapsed, and its toggle sits in the toolbar:

<!-- docs:snippet examples/chrome/src/lib.rs#inspector -->

Key equivalents come from the bindings instead of the menu items. `cx.bind_keys` registers
`secondary-n`, where `secondary` is Cmd on macOS and Ctrl on other platforms. `set_menus` builds
the menu bar and shows each item's shortcut from the same binding.

<!-- docs:snippet examples/chrome/src/lib.rs#menus -->

## Titlebar and window controls

A window with `Titlebar::Native` gets the OS titlebar, or the scaffold's toolbar inside it. For a
custom row, open the window with `Titlebar::Custom` and build the row with `titlebar()`. It
reserves the room that the OS controls need: the traffic lights on the left on macOS, the caption
buttons on the right on Windows, and GNOME's `button-layout` on Linux. On Hyprland, the compositor
draws no controls, so it reserves nothing. The reserved room updates for fullscreen and
right-to-left layouts.

<!-- docs:snippet examples/overrides/src/lib.rs#titlebar -->

`window_controls()` places the real buttons. With no argument, they take the slot that
`titlebar()` reserves. `.at(point(..))` pins them at window coordinates. Hello uses it to put the
traffic lights where its layout wants them:

<!-- docs:snippet examples/hello/src/lib.rs#controls -->

`.embedded()` makes the controls a layout slot instead. The backend moves the real buttons onto
the slot's rectangle on every layout pass, so they move with a sliding sidebar. The buttons stay
the OS's own, with their hover group, menu, and accessibility. On Linux, nd draws them only when the
window has client-side decorations, so the same code shows no buttons on Hyprland.

## Alerts, sheets and pickers

`window.alert` shows an `Alert` and calls back with the index of the button that the user
selected. On macOS, the alert is attached to the window as a sheet by default, and `.modal()` shows
it in front of the whole app. `window.open_dialog` opens the system file picker.

<!-- docs:snippet examples/chrome/src/lib.rs#delete-alert -->

Don't block in the callback. It runs after the dialog closes, while the app is free to handle
events.

## Settings

`open_settings` opens the app's settings window, or brings the open one to the front. On macOS,
the window has the system preferences layout: a toolbar of tabs over grouped rows. It opens with
`cmd-,` when the app binds `OpenSettings`. The tabs are a `toolbar().tab(..)`, and the rows are a
`form` of `form_section` and `form_row`:

<!-- docs:snippet examples/chrome/src/lib.rs#settings-tabs -->

For settings that persist to a file, `settings_form` builds the whole window from a settings
struct. For more information, see [settings and config](../guides/settings-and-config.md).

## Glass

`glass()` is a container that always maps to the system material, never to a replica. It takes
`.variant(GlassVariant::Regular | Clear)`, `.tint(color)`, and `.interactive()`. `glass_group()`
merges neighboring glass where the OS does. Like any other element, `glass()` accepts every
override. The Overrides example tints one:

<!-- docs:snippet examples/overrides/src/lib.rs#subtree -->

## Changing a pattern

If a pattern is wrong for your app, [Overrides](overrides.md#patterns) shows how to swap it for a
drawn version. If a pattern looks wrong on an OS, that's a bug in the backend. Fix it there, so
that every app gets the fix, instead of adding a margin to one app.
