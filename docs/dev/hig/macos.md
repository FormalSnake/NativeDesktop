# Apple HIG on macOS

The Human Interface Guidelines rules nd follows on macOS, with the value or the AppKit API that
supplies it and the file that does it. The rule of thumb behind all of them
([ARCHITECTURE.md](../../plan/ARCHITECTURE.md) 3.10): read the value from the OS or let the
system control draw itself, never a literal. `crates/nd-backend-appkit/tests/native_mappings_lint.rs`
fails on colour literals, metric constants and literal geometry in the backend.

Open gaps live in [docs/parity/macos-hig.md](../../parity/macos-hig.md). Values below were
measured on macOS 27 (2x, sidebar size Medium) unless they come from the HIG text.

## Text

[Typography](https://developer.apple.com/design/human-interface-guidelines/typography): body is
13 pt, nothing under 10 pt, no Dynamic Type. Use `NSFont.preferredFont(forTextStyle:)`:

| Style | Size | Weight |
| --- | --- | --- |
| Large Title | 26 | regular |
| Title 1 | 22 | regular |
| Title 2 | 17 | regular |
| Title 3 | 15 | regular |
| Headline | 13 | bold |
| Body | 13 | regular |
| Callout | 12 | regular |
| Subheadline | 11 | regular |
| Footnote, Caption | 10 | regular |

Small controls and secondary lines use `NSFont.smallSystemFontSize()` (11 pt). nd reads the styles
in `system_colors.rs`; the source list and palette set fonts from them.

## Colour and materials

- [Color](https://developer.apple.com/design/human-interface-guidelines/color): semantic
  `NSColor`s only (`labelColor`, `secondaryLabelColor`, `tertiaryLabelColor`, `separatorColor`,
  `controlAccentColor`), resolved at draw time so light, dark, Increase Contrast and the accent
  follow the user. `system_colors.rs` hands them to composed content as tokens.
- Sidebar counts and other trailing metadata use `tertiaryLabelColor` (Notes, Mail);
  `source_list.rs`.
- [Materials](https://developer.apple.com/design/human-interface-guidelines/materials): glass is
  for the control layer (toolbar, sidebar, palette, toasts), never content, never glass on glass.
  `NSGlassEffectView` on macOS 26 and later, `NSVisualEffectView` before; `glass.rs`, `palette.rs`,
  `toast.rs`.
- [Scroll views](https://developer.apple.com/design/human-interface-guidelines/scroll-views): no
  hairline or solid fill under the toolbar; the system scroll edge effect separates content from
  chrome. `edge.rs`, `scaffold.rs` (`NSTitlebarSeparatorStyle::None`).

## Window, toolbar, sidebar

- [Windows](https://developer.apple.com/design/human-interface-guidelines/windows) and
  [toolbars](https://developer.apple.com/design/human-interface-guidelines/toolbars): a real
  `NSToolbar` in the unified style with full-size content; system items for the sidebar toggle,
  flexible space and the sidebar tracking separator. `toolbar.rs`, `platform.rs`.
- [Sidebars](https://developer.apple.com/design/human-interface-guidelines/sidebars) and
  [split views](https://developer.apple.com/design/human-interface-guidelines/split-views):
  `NSSplitViewController` with `NSSplitViewItem(sidebarWithViewController:)`, so the system draws
  the material and the collapse slide (`animator().setCollapsed`); thin dividers
  (`NSSplitViewDividerStyle::Thin`). `scaffold.rs`, `split.rs`.
- Source lists: `NSOutlineView` in `NSTableViewStyle::SourceList` with
  `NSTableViewRowSizeStyle::Default`, so rows follow System Settings, Appearance, Sidebar icon
  size: Small 24 pt, Medium 32 pt, Large 40 pt, symbol and font scaled by AppKit. A table made in
  code starts at the custom 24 pt row, so nd sets the default style explicitly. Section headers
  are group rows at AppKit's own 19 pt with the space above them that the outline adds, as
  Finder's are. Icons are SF Symbols
  tinted with the accent (template images). `source_list.rs`, `icons.rs`.

## Controls

- [Buttons](https://developer.apple.com/design/human-interface-guidelines/buttons),
  [toggles](https://developer.apple.com/design/human-interface-guidelines/toggles),
  [sliders](https://developer.apple.com/design/human-interface-guidelines/sliders), pop-ups,
  segmented controls, fields: the AppKit controls themselves at their `fittingSize`
  (`crates/nd-backend-appkit/src/forms/`). Hit targets are at least 20 by 20 pt.
- Settings-style forms are SwiftUI's grouped `Form` (`swift/NDSwift/Sources/NDSwift/Form.swift`,
  `form.rs`). Switches in a form row are mini (`.controlSize(.mini)` on the hosted slot, 36 by 16 pt),
  as in System Settings; `Form.swift`, `forms/toggle.rs`. `NSSwitch` reports its regular size
  until it is in a window, so nd measures it in an offscreen one (`forms/mod.rs`). A standalone switch has its label before it.
- [Lists and tables](https://developer.apple.com/design/human-interface-guidelines/lists-and-tables):
  multicolumn tables alternate row backgrounds (`usesAlternatingRowBackgroundColors`), single
  column lists do not. `virtual_list.rs`.
- Empty states are SwiftUI's `ContentUnavailableView`; `status_page.rs`, `StatusPage.swift`.
- Alerts and sheets are `NSAlert` and `beginSheet`, popovers `NSPopover`: the system's
  presentation, dimming and keyboard (Return default, Escape and Command-period cancel).

## Motion

[Motion](https://developer.apple.com/design/human-interface-guidelines/motion): animate to show
a change, never for decoration, and honour Reduce Motion
(`NSWorkspace.accessibilityDisplayShouldReduceMotion`) with a fade instead of movement.

| What | How | File |
| --- | --- | --- |
| Sidebar collapse | `NSSplitViewItem` animator | `scaffold.rs` |
| Sidebar rows added or removed | `insertItems`/`removeItems` with fade plus slide | `source_list.rs` |
| List and table rows | `insertRows`/`removeRows` with fade | `virtual_list.rs` |
| Command palette | panel alpha 0 to 1 in 200 ms, out in 150 ms (`NSAnimationContext`) | `palette.rs` |
| Toasts | rise and fade, 220 ms in, 180 ms out | `toast.rs` |
| Sheets, popovers, alerts | system | |

## Interaction

- Keyboard: every control is an AppKit responder, so Tab, Full Keyboard Access, type-select in
  lists and the focus ring are the system's. Focus rings come from the control
  (`focusRingType` default), never drawn by nd.
- [Menus](https://developer.apple.com/design/human-interface-guidelines/the-menu-bar): standard
  roles (Hide, Quit, Undo, Cut, Copy, Paste, Minimize, Zoom, Enter Full Screen with Control-Command-F)
  map to AppKit selectors so the system validates and titles them. `services.rs`.
- [Context menus](https://developer.apple.com/design/human-interface-guidelines/context-menus)
  open on Control-click and secondary click; `views.rs`, `services.rs`.
- Scrolling: `NSScrollView` elasticity and momentum; nd never replaces them.

## Checking a change by eye

1. Build the example at utility priority: `CARGO_BUILD_JOBS=2 taskpolicy -c utility cargo build -p gallery`.
2. Run it with automation: `NATIVE_AUTOMATION=1 ND_AUTOMATION_SOCKET=<path> target/debug/gallery`.
3. Bring it to the front (`osascript -e 'tell application "System Events" to set frontmost of
   (first process whose unix id is <pid>) to true'`). An unfocused window dims its selection
   and controls and misleads the comparison.
4. Capture the window as the window server composites it: `screencapture -o -l <window id>`
   (glass only shows in such a capture).
5. Capture the first-party app with the same appearance and focus (Finder, Notes, System
   Settings), same scale, and compare element by element: row pitch, insets, text size and
   colour (sample pixels), control size, material.
6. Regenerate the goldens when the change is intended:
   `ND_UPDATE_GOLDENS=1 cargo nextest run --profile gate -p gallery -E 'test(/goldens/)'`, then
   look at every rewritten PNG.
