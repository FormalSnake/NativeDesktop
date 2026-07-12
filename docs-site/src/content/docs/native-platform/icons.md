---
title: Icons
description: freedesktop icon names are canonical; macOS maps a known subset to SF Symbols and passes everything else through.
---

`Button.iconName` and `MenuItem.iconName` (see the [Widget Reference](/components/widget-reference/))
both take a **freedesktop icon name** — the same vocabulary GTK/GNOME apps use — as the canonical,
cross-platform identifier. The mapping below is shared by both widgets; see
[Menu Bar](/native-platform/menu-bar/) for why `MenuItem.iconName` renders on macOS but is
intentionally invisible on GNOME.

## Linux: native, direct

On GTK, a freedesktop name resolves directly through the system icon theme — `gtk.Button.setIconName`
(icon-only) or an `adw.ButtonContent` pairing an icon with a label, with no translation step.

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

Only the names actually used by the framework's own examples are mapped today — this table grows as
new names are needed, not speculatively ahead of use.

If `iconName` isn't in the table, it's passed through **verbatim** as an SF Symbol name — so a direct
SF Symbol name (e.g. `"gearshape"`) works on macOS without needing an entry here. If neither the
mapping nor the direct name resolves to a real symbol, the macOS backend falls back to title-only
and prints an `ND_WARN unknown iconName` diagnostic rather than failing silently.
