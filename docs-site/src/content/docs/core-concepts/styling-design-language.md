---
title: Styling & Design Language
description: style is theme-neutral geometry; cssClasses reaches each platform's named design-language classes. Dark mode is automatic.
---

GTK styling is **not web CSS**, and NativeDesktop's `style` prop is deliberately not a CSS
reimplementation. There are two separate props, with two separate jobs.

## `style`: theme-neutral geometry

`style` covers layout and geometry only — never color-as-theme, never `flex`/`grid`/`position`.
Layout comes from container widgets (`<box>`, `<grid>`), not from `style`. The valid keys, generated
from `schema/widgets.json`:

| Key | Shape | Compiles to |
|---|---|---|
| `background` | color | GTK CSS `background-color` |
| `color` | color | GTK CSS `color` |
| `font` | object (`fontSize`, `fontWeight`, `fontFamily`) | GTK CSS, nested |
| `padding` | spacing | GTK CSS `padding` |
| `margin` | spacing | GTK widget property (`gtk_widget_set_margin_*`), not CSS |
| `hexpand` / `vexpand` | bool | GTK widget property |
| `halign` / `valign` | enum (`fill`/`start`/`end`/`center`) | GTK widget property |
| `border` | object (`borderWidth`, `borderColor`, `borderRadius`) | GTK CSS, nested |

`margin` differs from `padding` because GTK widget margins genuinely are widget properties, not CSS
— the schema encodes that distinction so codegen emits the right call on each backend. Unknown or
web-only keys (`display`, `justifyContent`, and the like) are rejected at the React renderer with a
Levenshtein fix-it message, and rejected again host-side as a defense-in-depth measure — a bad key
fails loudly at commit time.

The always-current key list lives at `docs/styling.md` (generated); this page summarizes it.

## `cssClasses`: reaching each platform's design language

`cssClasses?: string[]` is a set of named classes — borrowed from libadwaita's vocabulary — that map
onto real per-platform mechanisms, not onto CSS you write yourself:

- **On Linux**, each class is applied verbatim via `gtk_widget_add_css_class`. Additive-only: a
  class added on a prior update is never removed automatically.
- **On macOS**, a semantic subset maps onto real AppKit control properties, using dynamic system
  colors throughout so dark mode keeps working automatically:

  | Class(es) | AppKit mapping |
  |---|---|
  | `suggested-action` | `NSButton.bezelColor = .controlAccentColor`, default key equivalent |
  | `destructive-action` | `NSButton.bezelColor = .systemRed`, `hasDestructiveAction = true` |
  | `flat` | `NSButton.isBordered = false` |
  | `title-1`…`title-4` | `.preferredFont(forTextStyle:)` (`.largeTitle`/`.title1`/`.title2`/`.title3`) |
  | `heading` / `body` / `caption` / `caption-heading` | matching `preferredFont` text styles |
  | `dimmed` | `.textColor = .secondaryLabelColor` |
  | `monospace` / `numeric` | monospaced / monospaced-digit system font |

  Structural classes (`navigation-sidebar`, `card`, `toolbar`, `boxed-list`, `osd`, etc.) are
  silently ignored on macOS — that chrome comes from the `<splitview>`/`<headerbar>` widgets
  themselves, not from class strings.

## Dark mode is automatic

The Linux host runs as an `AdwApplication`, so `AdwStyleManager` tracks the system light/dark
preference from the first frame — unstyled widgets and `cssClasses` follow it with no app code
required. On macOS, the AppKit mappings above use dynamic system colors for the same reason. A
hardcoded `style.color`/`style.background` is an explicit override and does **not** adapt to dark
mode — prefer `cssClasses` plus the platform defaults for a theme-correct app.

`examples/notes/main.tsx` states this as a hard rule in its header comment: no color literals
anywhere in the app except one deliberate exception (a pinned-row accent border, chosen to read on
both themes) — every other visual is `cssClasses` plus the system's own styling.
