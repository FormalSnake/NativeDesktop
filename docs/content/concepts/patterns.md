+++
title = "Patterns"
description = "How window structure such as the toolbar, sidebar, inspector and titlebar is declared once and drawn by each OS."
+++

Declare the structure of a window with intents (`scaffold`, `toolbar`, `sidebar`, `inspector`,
`titlebar`) and let the OS draw it. On macOS each one is the system's own component, so a new
macOS release restyles the app with no change here. Do not rebuild a sidebar or toolbar from
`div`s; a pixel copy goes stale the day the OS changes. The Chrome example declares a Mail-style
window and names no AppKit class.

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

On Windows the drawn chrome follows WinUI, checked against Windows 11's own apps (Settings,
Task Manager): the sidebar and toolbar are one Mica surface with no divider, the content sits on
an opaque layer whose leading corner takes the window's radius, the sidebar draws like
NavigationView (Segoe Fluent icons, a subtle fill with an accent pill on the selected row) and its
toggle folds it to a 48 px icon rail, the caption buttons are as tall as the toolbar, the window
title is set in the 12 pt caption style, and `banner` is an inset card like InfoBar. Form
controls, dialogs and menus still use the Adwaita look there. Linux is checked on Hyprland.

## Scaffold, toolbar, sidebar and inspector

`scaffold()` takes a toolbar, a sidebar, the content and an inspector. Every part is optional.
The toolbar holds system items (`sidebar_toggle`, `inspector_toggle`, `flexible_space`) next to
your own buttons, and macOS gives them the system look and sizing.

<!-- docs:snippet examples/chrome/src/lib.rs#toolbar -->

The sidebar holds a `source_list`: sections of items with an icon and badge, the selection owned
by your state. On macOS it is an `NSOutlineView` in source-list style.

<!-- docs:snippet examples/chrome/src/lib.rs#sidebar -->

The inspector collapses from state and its toggle sits in the toolbar:

<!-- docs:snippet examples/chrome/src/lib.rs#inspector -->

Key equivalents come from the bindings, not from the menu items: `cx.bind_keys` registers
`secondary-n`, and `secondary` is cmd on macOS and ctrl elsewhere. `set_menus` builds the menu bar
and shows each item's shortcut from the same binding.

<!-- docs:snippet examples/chrome/src/lib.rs#menus -->

## Titlebar and window controls

A window with `Titlebar::Native` gets the OS titlebar, or the scaffold's toolbar in it. For a
custom row, open the window with `Titlebar::Custom` and build it with `titlebar()`, which reserves
the room the OS controls need: the traffic lights on the left on macOS, the caption buttons on
the right on Windows, GNOME's `button-layout` on Linux, and nothing on Hyprland, where the
compositor draws no controls. It updates on fullscreen and right-to-left layouts.

<!-- docs:snippet examples/overrides/src/lib.rs#titlebar -->

`window_controls()` places the real buttons. With no argument they take the slot `titlebar()`
reserves. `.at(point(..))` pins them at window coordinates, which Hello uses to put the traffic
lights where its layout wants them:

<!-- docs:snippet examples/hello/src/lib.rs#controls -->

`.embedded()` makes the controls a layout slot instead: the backend moves the real buttons onto
the slot's rectangle on every layout pass, so they ride along with a sliding sidebar. The buttons
stay the OS's, with their hover group, menu and accessibility. On Linux they are drawn only when
the window has client-side decorations, so the same code shows none on Hyprland.

## Alerts, sheets and pickers

`window.alert` shows an `Alert` and calls back with the index of the pressed button. By default it
is attached to the window as a sheet on macOS; `.modal()` shows it in front of the whole app.
`window.open_dialog` opens the system file picker.

<!-- docs:snippet examples/chrome/src/lib.rs#delete-alert -->

Do not block in the callback: it runs after the dialog closes, with the app free to handle
events.

## Settings

`open_settings` opens the app's settings window, or brings the open one forward. On macOS it is
the system preferences shape: a toolbar of tabs over grouped rows, opened with `cmd-,` when the
app binds `OpenSettings`. The tabs are a `toolbar().tab(..)`, the rows a `form` of
`form_section` and `form_row`:

<!-- docs:snippet examples/chrome/src/lib.rs#settings-tabs -->

For settings that persist to a file, `settings_form` builds the whole window from a settings
struct; see [settings and config](../guides/settings-and-config.md).

## Glass

`glass()` is a container that maps to the system material and never to a replica. It takes
`.variant(GlassVariant::Regular | Clear)`, `.tint(color)` and `.interactive()`, and `glass_group()`
merges neighbouring glass where the OS does. It takes every override like any other element. The
Overrides example tints one:

<!-- docs:snippet examples/overrides/src/lib.rs#subtree -->

## Changing a pattern

If a pattern is wrong for your app, [Overrides](overrides.md#patterns) shows how to swap one for a
drawn version. If it looks wrong on an OS, that is a bug in the backend: the fix goes there so
every app benefits, not in a per-app margin.
