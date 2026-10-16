+++
title = "Components"
description = "The built-in components by group, which are the OS's own control on macOS and which are drawn, and where to see each one."
+++

Every component is a function that returns an element, such as `button(..)`, `source_list(..)` or
`switch(..)`. Each maps to the OS's own control where one exists and to a drawn implementation
everywhere else, with the same props and events, so an app is written once. The
[component reference](reference.md) lists every constructor and builder method, generated from
`crates/nd-components`. This page is the map of what exists.

The Gallery shows one page per component. Pick one in its source list or press `secondary-K` for
the command palette:

<!-- docs:example gallery -->

## Navigation

- `command_palette`: a Spotlight-style palette. The app owns the query and the items and does the
  filtering; each keystroke reports the query. On macOS it is a glass card over the window.
- `source_list` and `source_tree`: sections of items with icons, badges and per-row actions. On
  macOS an `NSOutlineView` in source-list style, so open branches survive updates.
- `sidebar`, `inspector`, `toolbar`: window structure, covered in [Patterns](../concepts/patterns.md).
- `tab_strip` and `tab_view`: in-window tabs over a stack of pages. Windows that are tabs are a
  window option, see [windows and tabs](../guides/windows-and-tabs.md).

## Feedback

- `toast_overlay` and `banner`: transient and persistent messages. AppKit has neither, so both are
  drawn on every OS; macOS gets a glass capsule and VoiceOver announces each toast.
- `status_page`: an empty or error state with a symbol, title and description. SwiftUI's
  `ContentUnavailableView` on macOS when the app links Swift, a native stack otherwise.
- `progress` and `spinner`: determinate and indeterminate, the system indicators on macOS.
- `Alert`, `open_dialog` and `save_dialog`: the system alert (a sheet on the window) and file
  pickers. See [Patterns](../concepts/patterns.md#alerts-sheets-and-pickers).

## Forms and controls

- `button`, `icon_button`, `button_group`: the system bezel on macOS, adjustable with
  [overrides](../concepts/overrides.md).
- `text_field`, `secure_field`, `search_field`, `text_area`, `combo_box`: the
  AppKit text classes on macOS. Text is native-owned while it has focus, so IME and undo behave.
- `switch`, `checkbox`, `radio_group`, `segmented`, `select`, `slider`, `stepper`: the AppKit
  control of the same name. `select` can map to a pop-up, a segmented control or a radio group.
- `date_picker` and `color_well`: `NSDatePicker` and `NSColorWell` on macOS.
- `form`, `form_section`, `form_row`, `settings_group` and `settings_form`: grouped rows. A form
  scrolls itself; `form().title("Forms")` sets a page title that scrolls away with the rows, and
  `.pin_title()` keeps it above them, as Windows Settings does. See
  [settings and config](../guides/settings-and-config.md) for `settings_form`.

## Layout and windows

- `scaffold`, `titlebar`, `window_controls`, `glass`, `glass_group`: structure and materials, in
  [Patterns](../concepts/patterns.md).
- `split` and `split_pane`: an `NSSplitView` with the system divider, the ratio controlled by the
  app.
- `pane_tree`, `dock_view` and `tiles_view`: split-pane models for editors and tiled layouts, drawn
  on every OS.
- Virtual lists, tables, grids and trees for large data: see [virtualization](../guides/virtualization.md).

## Media and content

- `image` with a decode pipeline that sizes to the display and caches: layer contents on macOS.
- `player_bar`, `seek_bar` and `lyrics_view`: transport, a seek bar with a buffered range and
  synced lyrics. No platform has these, so they are painted from the theme on every OS.
- `shelf`: a horizontal scroller with the OS's momentum and rubber band on macOS.
- `composer`, `chat_bubble`, `chat_row`, `avatar` and `avatar_group`: messaging parts, painted from
  the theme.

## Native or drawn

On macOS, controls with an AppKit class use it, and the content you compose (`div`, `text`,
`canvas`, and components with no AppKit counterpart) is drawn with tokens resolved from the OS.
On Linux and Windows everything is drawn through GPUI with that platform's preset. A drawn
component still follows [theming](../concepts/theming.md), so light and dark, the accent and radii
are the same code path. If you need a component that is not here, define one with
[native widgets](../concepts/native-widgets.md).

Icons are one vocabulary, `Icon` from `nd-icons`, named after Lucide. macOS draws the matching SF
Symbol, Windows the matching Segoe Fluent Icons glyph (the font Windows 11's own apps use), and
Linux and the web the Lucide shape. An icon an app registers itself is drawn from its SVG where
no system glyph exists.
