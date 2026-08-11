---
title: Styling & Design Language
description: style is theme-neutral geometry; cssClasses reaches each platform's named design-language classes. Dark mode is automatic.
---

GTK styling is not web CSS, and NativeDesktop's `style` prop is deliberately not a CSS
reimplementation. There are two separate props, with two separate jobs.

## `style`: theme-neutral geometry

`style` covers layout and geometry only, never color-as-theme and never `flex`/`grid`/`position`.
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

`margin` differs from `padding` because GTK margins genuinely are widget properties rather than CSS.
The schema encodes that distinction so codegen emits the right call on each backend. Unknown or
web-only keys like `display` and `justifyContent` are rejected at the React renderer with a
Levenshtein fix-it message, and rejected again host-side, so a bad key fails loudly at commit time.

The always-current key list is the generated `docs/styling.md`. This page summarizes it.

## `cssClasses`: reaching each platform's design language

`cssClasses?: string[]` is a set of named classes, borrowed from libadwaita's vocabulary, that map
onto real per-platform mechanisms rather than CSS you write yourself.

On Linux the class list is reconciled as a set on every update: each allowlisted class is added when
requested and removed when no longer requested, so classes don't accumulate across renders.
Container-scoped classes (`navigation-sidebar`, `boxed-list`, `boxed-list-separate`, `menu`,
`inline`) style nothing when applied to a widget type libadwaita doesn't target with them — the host
prints a one-time `ND_WARN` naming the class and widget type instead of failing silently; reach for
the structural widgets (`<sourcelist>`, `<sourcetree>`, `<settingsgroup>`) when you want that
chrome.

On macOS a semantic subset maps onto real AppKit control properties, using dynamic system colors
throughout so dark mode keeps working:

| Class | AppKit mapping |
|---|---|
| `suggested-action` | `NSButton.bezelColor = .controlAccentColor`, default key equivalent |
| `destructive-action` | `NSButton.bezelColor = .systemRed`, `hasDestructiveAction = true` |
| `flat` | `NSButton.isBordered = false` |
| `pill` | `NSButton.borderShape = .capsule`; on a `<label>`, a capsule quaternary-fill count badge |
| `activatable` (on a `<box>`) | native hover feedback: a quaternary-fill highlight at the concentric radius |
| `title-1` through `title-4` | `.preferredFont(forTextStyle:)` with `.largeTitle`, `.title1`, `.title2`, `.title3` |
| `heading`, `body`, `caption`, `caption-heading` | matching `preferredFont` text styles |
| `dimmed` | `.textColor = .secondaryLabelColor` |
| `monospace`, `numeric` | monospaced and monospaced-digit system font |
| `toolbar` (on a `<box>`) | `NSVisualEffectView` `.headerView` backing plus a 1 pt bottom hairline |
| `boxed-list` (on a `<box>`) | native grouped `NSBox` card with inset hairline dividers |

The remaining structural classes (`navigation-sidebar`, `card`, `osd`) are ignored on macOS. That
chrome comes from the `<splitview>` and `<headerbar>` widgets themselves rather than from class
strings.

Beyond classes, three Button props carry state the platform renders natively: `prominent` maps to
the accent treatment (`suggested-action` on GTK; an accent bezel, or a `.prominent` toolbar item
once the button is promoted into the window toolbar, on macOS), `badge` renders a count next to the
control (`NSItemBadge` on a promoted toolbar item; a capsule suffix label on GTK), and `size` picks
the control metrics (`NSControl.controlSize`; compact/large CSS metrics on GTK). Window-level
`density="compact"` opts the whole window into compact control metrics
(`prefersCompactControlSizeMetrics` on macOS, a tightened CSS block on GTK).

## Dark mode is automatic

The Linux host runs as an `AdwApplication`, so `AdwStyleManager` tracks the system light and dark
preference from the first frame. Unstyled widgets and `cssClasses` follow it with no app code. On
macOS the AppKit mappings above use dynamic system colors for the same reason. A hardcoded
`style.color` or `style.background` is an explicit override and does not adapt. Prefer `cssClasses`
plus the platform defaults for a theme-correct app.

`examples/notes/main.tsx` holds itself to this as a hard rule in its header comment: no color
literals anywhere except one deliberate exception, a pinned-row accent border chosen to read on both
themes. Everything else is `cssClasses` plus the system's own styling.

An app that needs the live accent color (a status dot, a chart series) reads it from
`system.getAppearance()` / `system.onAppearanceChange()`, which return
`{ appearance: "light" | "dark", accentColor: "#rrggbb" }` — the AdwStyleManager accent on Linux,
`NSColor.controlAccentColor` on macOS — instead of hardcoding a hex.

## Spacing scale

For bespoke layout (`<box spacing>`, `style.padding`), `@nativedesktop/react` exports the platform's
design-language scale instead of magic numbers:

```tsx
import { Spacing, ContentMargin } from "@nativedesktop/react";

<box spacing={Spacing.sm} style={{ padding: ContentMargin }} />;
```

`Spacing` is `{ xs, sm, md, lg, xl }` — `3/6/12/18/24` on Linux (GNOME's multiples of 6),
`4/8/12/20/24` on macOS — and `ContentMargin` is the standard window-edge margin (`12` / `20`).
The structural widgets (`<settingsgroup>`, `<row>`, `<clamp>`, `<sourcelist>`) carry native metrics
themselves and need none of this.
