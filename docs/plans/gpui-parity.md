# gpui-component parity, natively

Target: https://longbridge.github.io/gpui-component/ (~85 components). We have 57
widgets plus the non-widget app APIs. This file tracks the gap and the waves.

Rule that shapes every line below: gpui owns its pixels, we do not. A gpui
component that only exists because it had to draw its own chrome (WindowBorder,
custom scrollbars, FocusTrap, Theme registry) is already covered by the OS on
both backends and is NOT a gap. A component that is real UI vocabulary is.

## Already covered

| gpui | ours |
| --- | --- |
| Button, ButtonGroup, DropdownButton, Toggle | Button, SegmentedControl / `linked`, MenuButton + SplitButton, ToggleButton |
| Checkbox, Radio, Switch, Slider, Select | same names |
| Input, Textarea, NumberInput, SearchInput | TextInput, TextArea, NumberInput, SearchInput |
| ColorPicker, DatePicker, Calendar | ColorPicker, DatePicker (`displayStyle="calendar"`) |
| Table, DataTable, List, VirtualList, Tree | Table, ListView, TreeView, SourceTree (all natively virtualized) |
| Separator, ScrollArea, Resizable, GroupBox | Separator, ScrollView, Paned + `@nativedesktop/panes`, SettingsGroup |
| Sidebar, TitleBar, StatusBar | SourceTree + SplitView, HeaderBar, ToolbarView bottom slot |
| Tab/TabBar, Menu, NativeMenu, Link | TabView, Menu/MenuItem/Menubar, LinkButton |
| Alert, AlertDialog, Popover, Toast, Progress, Spinner | Banner, showAlert, Popover, ToastOverlay, ProgressBar, Spinner |
| Icon, Label, Settings page, Clipboard, Root, Inspector | Image(iconName), Label, SettingsGroup/Row/SwitchRow/Clamp, system.clipboard, render(), examples/inspector |
| Theme, Sizing, GlobalState, FocusTrap, IndexPath | style + cssClasses + accentColor, Button.size, React state, native focus, n/a |

## Wave 1 — native widgets, both backends

Owner: `schema/widgets.json` + `tools/codegen.ts` (Zig create/apply/signal +
Swift create/apply/signal). One agent owns the file at a time; codegen.ts is a
single 6k-line file and concurrent edits conflict.

| gpui | plan | GTK | AppKit |
| --- | --- | --- | --- |
| Avatar | new `<avatar>` | AdwAvatar | NSImageView + rounded mask |
| Badge | new `<badge>` | GtkLabel, `.badge` | NSTextField pill |
| Tag | new `<tag>` | GtkLabel, `.pill` | NSTextField pill |
| Kbd | new `<kbd>` | GtkLabel, `.monospace` + frame | NSTextField + bezel |
| ComboBox | new `<combobox>` | GtkDropDown `enable-search` | NSComboBox |
| Breadcrumb | new `<breadcrumb>` | GtkBox of flat buttons | NSPathControl |
| Tooltip | universal `tooltip` prop | `gtk_widget_set_tooltip_text` | `view.toolTip` |
| Rating | `style` enum on LevelIndicator | star row | NSLevelIndicator `.rating` |

## Wave 2 — composition layer, `@nativedesktop/ui`

Pure TS over existing widgets, no schema or ABI change — the `@nativedesktop/panes`
precedent. Touches only `packages/ui/**`, so it runs parallel to wave 1.

Accordion, DescriptionList, Pagination, Stepper, HoverCard, SearchableList,
Form + FormField (validation state), OtpInput, ButtonGroup, StatusBar,
AvatarGroup (after wave 1's Avatar).

## Wave 3 — overlays and rich text

| gpui | plan |
| --- | --- |
| Dialog (arbitrary content) | real gap. NSWindow sheet / AdwDialog. New `<dialog>` widget |
| Sheet / Drawer | AdwDialog bottom sheet / NSPanel slide-in |
| Text / TextView (Markdown, selection) | NSTextView attributed / GtkTextView Pango |
| ProgressCircle | determinate ring, custom draw both sides |
| Skeleton | shimmer, CSS animation / CALayer |
| Table column resize, multi-select | prop additions on Table |

## Wave 4 — the three big ones, native on each side

No deferrals. Each has a real native idiom per platform; they are just bigger.

| gpui | GTK | AppKit |
| --- | --- | --- |
| Chart family + Plot | `GtkDrawingArea` + Cairo/`GtkSnapshot` (what GNOME apps actually do) | SwiftUI **Swift Charts** hosted in `NSHostingView` |
| Editor | **GtkSourceView** (GNOME Builder / Text Editor use it), dlopen'd per the no-link rule | `NSTextView` + TextKit 2 attributed highlighting |
| Dock / Tiles | drag via `GtkDragSource`/`GtkDropTarget` | drag via `NSDraggingSource`/`NSDraggingDestination` |

Dock needs one new capability first: widget-level drag and drop. Today only
`app.onFileDrop` exists. Add `draggable` / `dropTarget` props plus
`dragStarted` / `dragEnded` / `dropped` events, then Dock and Tiles are a model
on top of `@nativedesktop/panes` and the existing tab system.

LSP is deliberately NOT inside the editor widget — a language server belongs in
app-side TS, and the widget takes `diagnostics` / `decorations` as props.

## Ordering

`schema/widgets.json` and `tools/codegen.ts` are single files that every native
wave edits, so native waves run one at a time. Pure-TS work runs alongside them.

1. Wave 1 native widgets, and `@nativedesktop/ui` (parallel)
2. Wave 3 overlays + display: Dialog, Sheet, RichText, ProgressCircle, Skeleton, Table column resize
3. Wave 4 Chart, CodeEditor, drag and drop
4. Dock / Tiles model on `@nativedesktop/panes`, wired to the DnD from step 3

## Status: done

Widgets 57 -> 70. All gates green as of the final sweep:

| Gate | Result |
| --- | --- |
| `bun tools/codegen.ts` | deterministic across two runs |
| `zig build` / `zig build test` | exit 0 |
| `scripts/mac/build-appkit-host.sh` | exit 0 (libnd rebuilt and repacked) |
| `bun test packages` | 308 pass, 0 fail |
| `bun scripts/parity-drive.ts` | `ND_PARITY_OK`, 14 sections |
| `bunx astro build` | 54 pages |
| Screenshots | 32 AppKit + 32 GTK, component-cropped |

### Landed

- **13 native widgets**: avatar, badge, tag, kbd, combobox, breadcrumb, dialog,
  sheet, richtext, progresscircle, skeleton, chart (6 types), codeeditor.
- **Universal props**: `tooltip`, `enabled`, `draggable`/`dragPayload`/
  `dropTarget`, plus a `focus` command and TabView `selectionChanged`.
- **`@nativedesktop/ui`**: Accordion, DescriptionList, Pagination, Stepper,
  HoverCard, SearchableList, Form + FormField, OtpInput, ButtonGroup, StatusBar.
- **Dock and Tiles** on `@nativedesktop/panes`, drag-wired through the new DnD.
- **AppKit leaves on SwiftUI** via `NDHostedLeaf`, so Apple owns the chrome.
  Radio and LevelIndicator stayed on AppKit deliberately: SwiftUI has no
  composable radio group on macOS, and no equivalent of NSLevelIndicator's four
  styles.
- `examples/parity` (14 sections) and 12 documented pages with paired shots.

### Deliberate platform differences

Each backend matches its own OS rather than the other backend.

- `<sheet>` `edge` and `<dialog>` `title` are inert on AppKit. macOS presents
  every sheet from the window and draws no titlebar for one. GTK honours both
  through AdwDialog's bottom-sheet presentation.
- AppKit drags starting in another application do not reach `dropped`. External
  file drags stay on `app.onFileDrop`.
- GTK answers `-32003 inputUnsupported` for synthesized input, so drive scripts
  use semantic click/setValue there.
- A disabled container dims on AppKit but still routes clicks to its children;
  `gtk_widget_set_sensitive` propagates through a subtree and AppKit has no
  view-level equivalent.

### Traps worth remembering

- **`zig build` does not rebuild `libnd.a`.** A stale archive silently makes new
  widgets report `role: null` at runtime while every build looks green. Use
  `scripts/mac/build-appkit-host.sh`.
- Swift builds on this machine need `env -u SDKROOT -u DEVELOPER_DIR`; the nix
  devshell otherwise pins a 14.4 SDK against the 6.3 compiler. `bun test
  packages` needs them SET, since `xcrun actool` runs in the icon tests.
- Do not call `fittingSize` inside `intrinsicContentSize`. It recurses and
  segfaults. `NDBoxStackView` computes its size from arranged subviews instead.
- A screenshot crop is verified by opening it, not by checking its dimensions.
  A wrong-region crop has a perfectly plausible size.

### Not built

Radar and Sankey charts. Every other gpui-component surface is covered, by a
native widget, a composed component, or a documented platform difference.
