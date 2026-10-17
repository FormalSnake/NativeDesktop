+++
title = "Components"
description = "The built-in components by group: which ones are the OS's own control on macOS, which ones nd draws, and where to see each one."
+++

Every component is a function that returns an element, such as `button(..)`, `source_list(..)`,
or `switch(..)`. Each one maps to the OS's own control where one exists, and to a drawn
implementation everywhere else, with the same props and events, so you write your app once. The
[component reference](reference.md) lists every constructor and builder method, generated from
`crates/nd-components`. This page is a map of what exists.

The Gallery shows one page per component. Select a component in its source list, or press
`secondary-K` to open the command palette:

<!-- docs:example gallery -->

## Navigation

- `command_palette`: a Spotlight-style palette. The app owns the query and the items, and does the
  filtering. Each keystroke reports the query. On macOS, it's a glass card over the window.
- `source_list` and `source_tree`: sections of items with icons, badges, and per-row actions. On
  macOS, they're an `NSOutlineView` in source-list style, so open branches survive updates.
- `sidebar`, `inspector`, `toolbar`: window structure, covered in [Patterns](../concepts/patterns.md).
- `tab_strip` and `tab_view`: in-window tabs over a stack of pages. Windows that are tabs are a
  window option. See [windows and tabs](../guides/windows-and-tabs.md).

## Feedback

- `toast_overlay` and `banner`: transient and persistent messages. AppKit has neither, so nd draws
  both on every OS. On macOS, you get a glass capsule, and VoiceOver announces each toast.
- `status_page`: an empty or error state with a symbol, title, and description. On macOS, it's
  SwiftUI's `ContentUnavailableView` when the app links Swift, and a native stack otherwise.
- `progress` and `spinner`: determinate and indeterminate progress. On macOS, they're the system
  indicators.
- `Alert`, `open_dialog`, and `save_dialog`: the system alert (a sheet on the window) and file
  pickers. See [Patterns](../concepts/patterns.md#alerts-sheets-and-pickers).

## Forms and controls

- `button`, `icon_button`, `button_group`: the system bezel on macOS, adjustable with
  [overrides](../concepts/overrides.md).
- `text_field`, `secure_field`, `search_field`, `text_area`, `combo_box`: the
  AppKit text classes on macOS. The native control owns the text while it has focus, so IME and
  undo work as expected.
- `switch`, `checkbox`, `radio_group`, `segmented`, `select`, `slider`, `stepper`: the AppKit
  control of the same name. `select` can map to a pop-up, a segmented control, or a radio group.
- `date_picker` and `color_well`: `NSDatePicker` and `NSColorWell` on macOS.
- `form`, `form_section`, `form_row`, `settings_group`, and `settings_form`: grouped rows. A form
  scrolls itself. `form().title("Forms")` sets a page title that scrolls away with the rows, and
  `.pin_title()` keeps the title above them, as Windows Settings does. For `settings_form`, see
  [settings and config](../guides/settings-and-config.md).

## Layout and windows

- `scaffold`, `titlebar`, `window_controls`, `glass`, `glass_group`: structure and materials, in
  [Patterns](../concepts/patterns.md).
- `split` and `split_pane`: an `NSSplitView` with the system divider. The app controls the ratio.
- `pane_tree`, `dock_view`, and `tiles_view`: split-pane models for editors and tiled layouts,
  drawn on every OS.
- Virtual lists, tables, grids, and trees for large data: see [virtualization](../guides/virtualization.md).

## Media and content

- `image`: an image with a decode pipeline that sizes to the display and caches the result. On
  macOS, it's layer contents.
- `player_bar`, `seek_bar`, and `lyrics_view`: transport controls, a seek bar with a buffered
  range, and synced lyrics. No platform has these, so nd paints them from the theme on every OS.
- `shelf`: a horizontal scroller. On macOS, it has the OS's momentum and rubber band.
- `composer`, `chat_bubble`, `chat_row`, `avatar`, and `avatar_group`: messaging parts, painted
  from the theme.

## Native or drawn

On macOS, controls that have an AppKit class use it. nd draws the content that you compose (`div`,
`text`, `canvas`, and components with no AppKit counterpart) with tokens resolved from the OS. On
Linux and Windows, nd draws everything through GPUI with that platform's preset. A drawn component
still follows [theming](../concepts/theming.md), so light and dark mode, the accent, and radii go
through the same code path. If you need a component that isn't here, define one with
[native widgets](../concepts/native-widgets.md).

Icons share one vocabulary: `Icon` from `nd-icons`, named after Lucide. macOS draws the matching
SF Symbol, and Windows draws the matching Segoe Fluent Icons glyph (the font that Windows 11's own
apps use). Linux and the web draw the Lucide shape. When an app registers its own icon and no
system glyph exists, nd draws the icon from its SVG.
