# macOS HIG parity

What nd's AppKit output still does differently from Apple's own apps on macOS 27, found by
capturing the gallery next to Finder, Notes and System Settings at the same scale (2x, dark
appearance, sidebar size Medium, focused windows) and reading the HIG pages for each component.
Most controls are AppKit's or SwiftUI's own (toolbar, split view, buttons, fields, switches,
segmented controls, status page, alerts, sheets, popovers), so the gaps sit where nd draws, lays
out or animates something itself.

A ticked item is fixed in nd for every app; the note says how it was checked.

## Sidebar (source list)

- [x] Rows used AppKit's custom 24 pt row of a table made in code. Finder and Notes follow the
  sidebar size in System Settings (Medium is 32 pt, with a larger symbol). The HIG: row height and
  glyph size come from that setting. Checked: gallery capture next to Finder, both 64 px rows at
  2x with the same symbol size.
- [x] Section titles sat in the middle of a full-height row. Finder's are AppKit's 19 pt group
  row with spacing above, against the rows they head. Checked: side by side with Finder, header to
  first row offset matches.
- [x] Counts were drawn in the label colour, as bright as the titles. Notes draws them in the
  tertiary label colour. Checked: "All folders" count in the gallery capture.
- [x] Adding or removing a row reloaded the whole outline, so rows jumped. Finder slides a row in
  and out. Checked: `source_list` unit tests for the diff, and Add folder / delete in the gallery
  insert and remove in place with the right rows left.

## Command palette

- [x] The palette appeared and vanished in one frame. Spotlight's panel fades in and out
  (200 ms in, 150 ms out). Checked: `nav_components` tests; unmount still removes it at once.

- [x] With the palette open, leaving the app and coming back made the window key, so typing
  missed the palette's field. Spotlight keeps the keyboard. The palette's panel now takes key
  back when its window turns key. Checked: `nav_components`, which fails with the handback off.

## Forms and tables

- [x] Switches in a form row were regular (54 by 24 pt). System Settings uses mini ones (36 by
  16 pt). Checked: automation tree reports 36 by 16 and the Forms page capture next to Settings.
- [x] Multicolumn tables had no alternating rows; the HIG and Finder's list view alternate them.
  `virtual_list.rs` turns them on for non-inset tables only. Checked: `list_kinds` table test.

## Menus

- [x] The View menu item was an app's "Toggle Sidebar" on Option-Command-S. The HIG wants "Show
  Sidebar" / "Hide Sidebar" flipping with the state, on Control-Command-S. Now
  `MenuRole::ToggleSidebar`, with `nd::ToggleSidebar` for the key and palette elsewhere.

## Gallery (the example itself, not the framework)

- [x] Standalone switches had their label after the switch. The HIG and Settings put the label
  first and the switch at the trailing edge. Checked: Tabs page capture.

## Not changed

- Document tabs (`TabStyle::Strip`) show as one segmented control across the width: macOS has
  no in-window document tab bar (its document tabs are window tabs), see `nd-components/src/tabs.rs`.
- The split view page marks the focused pane with an accent border: that is the gallery's own
  drawing, a demo of pane focus, not a framework default.
- Trackpad scrolling in FormalMusic (glitchy vertical and horizontal momentum) and the seek bar's
  track belong to the formalmusic-ui lane, which has `media/seek.rs` and `virtual_list.rs` open.
