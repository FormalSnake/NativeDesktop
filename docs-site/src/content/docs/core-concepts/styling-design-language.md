---
title: Styling & Design Language
description: style is theme-neutral geometry; cssClasses reaches each platform's named design-language classes. Dark mode is automatic.
---

GTK styling is not web CSS, and the `style` prop is not a CSS reimplementation. Two props, two
jobs: `style` covers geometry, `cssClasses` reaches each platform's design language.

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

The generated `docs/styling.md` carries the always-current key list. This page summarizes it.

## `cssClasses`: reaching each platform's design language

`cssClasses?: string[]` is a set of named classes borrowed from libadwaita's vocabulary. They map
onto real per-platform mechanisms rather than CSS you write yourself.

On Linux the class list is reconciled as a set on every update: each allowlisted class is added when
requested and removed when no longer requested, so classes never accumulate across renders.
Container-scoped classes (`boxed-list`, `boxed-list-separate`, `menu`, `inline`) style nothing when
applied to a widget type libadwaita does not target with them. The host prints a one-time `ND_WARN`
naming the class and widget type; reach for the structural widgets (`<sourcelist>`, `<sourcetree>`,
`<settingsgroup>`) when you want that chrome.

Three classes libadwaita scopes to other widget types are carried by framework base CSS so they mean
the same thing on both backends: `pill` on a `<label>` is a capsule count badge (libadwaita treats
it as a button size class), `activatable` on a `<box>` is row hover feedback (libadwaita scopes it
to `row`), and `navigation-sidebar` on a `<box>` gives its `<button>` children libadwaita's own
sidebar-row metrics and states (libadwaita scopes those to `row`, which cannot exist inside a box).

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
| `navigation-sidebar` (on a `<box>`) | a `.sourceList` `NSTableView` backing the box, when its children are row-shaped |

`navigation-sidebar` on a box of flat `<button>` rows puts a real source-list table behind the box
on macOS, so accent-when-key selection, row metrics, and row insets come from AppKit instead of
your styling. The takeover is gated on the children being row-shaped: the table covers the whole
box, and a composite row (button plus caption plus badge) would lose everything that is not the
button. `nd-native-sidebar` on the same box skips that gate and takes over unconditionally.

The class is portable. On GTK the same box gets libadwaita's sidebar-row metrics and states through
framework base CSS. Both backends read `suggested-action` on a row as the selection signal and
neither paints it as an accent call to action inside a sidebar, so one tree gives you a native
sidebar on each platform with no per-row padding. `<sourcelist>` and `<sourcetree>` are still the
better choice when the rows are data rather than a fixed handful of destinations.

The remaining structural classes (`card`, `osd`) are ignored on macOS, where that chrome comes from
the `<splitview>` and `<headerbar>` widgets themselves.

Beyond classes, three Button props carry state the platform renders natively: `prominent` maps to
the accent treatment (`suggested-action` on GTK; an accent bezel, or a `.prominent` toolbar item
once the button is promoted into the window toolbar, on macOS), `badge` renders a count next to the
control (`NSItemBadge` on a promoted toolbar item; a capsule suffix label on GTK), and `size` picks
the control metrics (`NSControl.controlSize`; compact/large CSS metrics on GTK). Window-level
`density="compact"` opts the whole window into compact control metrics
(`prefersCompactControlSizeMetrics` on macOS, a tightened CSS block on GTK).

## Dark mode is automatic

The Linux host runs as an `AdwApplication`, so `AdwStyleManager` tracks the system light and dark
preference from the first frame. Unstyled widgets and `cssClasses` follow it with no app code. The
macOS mappings above use dynamic system colors for the same reason. A hardcoded `style.color` or
`style.background` is an explicit override and does not adapt.

`examples/notes/main.tsx` holds to this as a hard rule: no color literals anywhere except one
pinned-row accent border chosen to read on both themes.

For a live accent color (a status dot, a chart series), read `system.getAppearance()` or
`system.onAppearanceChange()`, which return
`{ appearance: "light" | "dark", accentColor: "#rrggbb" }`: the AdwStyleManager accent on Linux,
`NSColor.controlAccentColor` on macOS.

## Spacing scale

For bespoke layout (`<box spacing>`, `style.padding`), `@nativedesktop/react` exports the platform's
design-language scale instead of magic numbers:

```tsx
import { Spacing, ContentMargin, ContentWidth } from "@nativedesktop/react";

<box spacing={Spacing.sm} style={{ padding: ContentMargin }} />;
```

`Spacing` is `{ xs, sm, md, lg, xl }`: `3/6/12/18/24` on the GTK backend (GNOME's multiples of 6),
`4/8/12/20/24` on AppKit. `ContentMargin` is the standard window-edge margin, `12` on GTK and `20`
on AppKit. All three are keyed on `Platform.backend`, not `Platform.os`, so the GTK backend running
on macOS via Quartz still lays out GNOME's numbers. See [Which backend is
drawing](/core-concepts/architecture/#which-backend-is-drawing). They resolve after `render()`'s
handshake (`Spacing`'s fields are live getters, `ContentMargin` and `ContentWidth` plain bindings
re-assigned once the backend is known) and fall back to the OS convention before that. The
structural widgets (`<settingsgroup>`, `<row>`, `<clamp>`, `<sourcelist>`) carry native metrics
themselves.

## Content column

`ContentWidth` is the platform's reading measure for settings-shaped content — `600` on GTK
(AdwClamp's own default), `720` on AppKit. Give it to a `<clamp>` so the whole column shares one
width:

```tsx
<scrollview>
  <clamp maximumSize={ContentWidth}>
    <box orientation="vertical" style={{ padding: ContentMargin }}>
      <settingsgroup title="Code Editor">…</settingsgroup>
      <codeeditor text={code} language="typescript" />
    </box>
  </clamp>
</scrollview>
```

The cap belongs to the column, not to any one widget. On AppKit `<settingsgroup>` is a SwiftUI
grouped `Form`, and that style caps and centers its own card inside whatever width it is handed —
a `<codeeditor>` or `<table>` beside it has no such cap, so in a wide pane the two disagree, and
nothing you can pass the group turns its cap off. Clamping the column below the cap settles it:
the Form stops capping, its card fills the column, and its siblings line up with it. GTK behaves
the same way natively, since AdwPreferencesGroup fills its AdwClamp. Leave canvases (a chart grid,
a table, a board) outside the clamp so they still get the whole pane.
