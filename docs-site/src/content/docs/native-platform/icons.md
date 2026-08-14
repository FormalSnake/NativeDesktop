---
title: Icons
description: freedesktop icon names are canonical; macOS maps a known subset to SF Symbols and passes everything else through.
---

`Button.iconName` and `MenuItem.iconName` both take a freedesktop icon name, the vocabulary GTK and
GNOME apps use, as the canonical cross-platform identifier. Both widgets share the mapping below.
See [Menu Bar](/native-platform/menu-bar/) for why `MenuItem.iconName` renders on macOS but stays
invisible on GNOME.

## Linux: native, direct

On GTK, a freedesktop name resolves directly through the system icon theme: `gtk.Button.setIconName`
for icon-only buttons, or an `adw.ButtonContent` that pairs an icon with a label. There is no
translation step.

## macOS: mapped to SF Symbols, with pass-through

`swift/Sources/NDShell/Icons.swift` maps a known subset of freedesktop names to their closest SF
Symbol equivalent:

| freedesktop name | SF Symbol |
|---|---|
| `list-add` | `plus` |
| `document-new` | `square.and.pencil` |
| `edit-delete` / `user-trash` | `trash` |
| `edit-find` / `system-search` | `magnifyingglass` |
| `view-list` | `list.bullet` |
| `checklist` | `checklist` |
| `mail-send` | `paperplane` |
| `document-open` | `folder` |
| `emblem-shared` | `person.crop.circle` |
| `go-previous` | `chevron.backward` |
| `go-next` | `chevron.forward` |
| `window-close` | `xmark` |
| `document-save` | `square.and.arrow.down` |
| `view-refresh` | `arrow.clockwise` |
| `open-menu` | `ellipsis.circle` |
| `view-pin` / `pin` | `pin` |
| `starred` | `star.fill` |
| `non-starred` | `star` |
| `edit-copy` | `doc.on.doc` |
| `edit-cut` | `scissors` |
| `edit-paste` | `doc.on.clipboard` |
| `edit-undo` / `edit-redo` | `arrow.uturn.backward` / `arrow.uturn.forward` |
| `go-up` / `go-down` / `go-home` | `chevron.up` / `chevron.down` / `house` |
| `pan-up` / `pan-down` / `pan-start` / `pan-end` | matching `chevron.*` |
| `network-offline` | `wifi.slash` |
| `process-stop` | `xmark.octagon` |
| `zoom-in` / `zoom-out` / `zoom-original` | `plus.magnifyingglass` / `minus.magnifyingglass` / `1.magnifyingglass` |
| `media-playback-start` / `-pause` / `-stop` | `play.fill` / `pause.fill` / `stop.circle` |
| `preferences-system` / `emblem-system` | `gearshape` |
| `dialog-information` / `-warning` / `-error` / `-question` | `info.circle` / `exclamationmark.triangle` / `exclamationmark.triangle` / `questionmark.circle` |
| `folder-new` / `user-home` / `bookmark-new` | `folder.badge.plus` / `house` / `bookmark` |
| `document-edit` / `document-print` | `pencil` / `printer` |
| `changes-prevent` / `changes-allow` | `lock` / `lock.open` |

The full table lives in `swift/Sources/NDShell/Icons.swift` (roughly 90 names); it grows as new
names are needed.

If `iconName` isn't in the table, it passes through verbatim as an SF Symbol name, so a direct
SF Symbol name (e.g. `"gearshape"`) works on macOS without an entry here. If neither the
mapping nor the direct name resolves to a real symbol, the macOS backend falls back to title-only
and prints an `ND_WARN unknown iconName` diagnostic instead of failing silently.

## Symbol configuration

Resolved SF Symbols ship configured rather than bare. A `<button iconName>` derives its symbol's
point size from the button font, uses the `.large` scale when icon-only and `.medium` next to a
label, and prefers hierarchical rendering, per the HIG guidance for control glyphs. `<image>`
exposes the same axes as create-only props: `symbolScale` (`small`/`medium`/`large`), `symbolWeight`
(`regular`/`medium`/`semibold`/`bold`), and `symbolRenderingMode`
(`monochrome`/`hierarchical`/`multicolor`). On GTK, `symbolScale` maps to the icon pixel size;
weight and rendering mode have no GTK peer, since symbolic icons carry one stroke weight and recolor
via CSS, so they are inert there.

## macOS 27 hides menu-item symbol images by default

Starting with macOS 27, `NSMenu` hides menu-item symbol images unless the item opts in, following a
revised HIG that wants menu icons used sparingly. `MenuItem.iconName` is advisory there: the system
may not show it. An item that must keep its image sets `iconVisible` alongside `iconName`, which
maps to `NSMenuItem.preferredImageVisibility = .visible` on macOS 27 and is a no-op on earlier
releases, where images still render by default.
