# Styling pointer

GTK styling is not web CSS. The authoritative, always-current key list is generated at
[`docs/styling.md`](../styling.md); read that, not this file, for the actual `style` prop schema.

Do not hallucinate `flex`, `grid`, `position`, `display`, or `justifyContent`; none of these exist
on the `style` prop. Layout comes from container widgets (`<box>`/`<grid>`), never from `style`.
Unknown or web-only keys are rejected at the React renderer with a Levenshtein fix-it message
(`validateStyle`) and defensively rejected host-side too, so a bad key fails loudly at commit time
rather than silently doing nothing.

This file is intentionally short and is not kept in sync with schema changes by hand. Treat
`docs/styling.md` as the source of truth and this file as the "start here" pointer to it.

## cssClasses across platforms

`cssClasses?: string[]` is validated on the React side against an Adwaita/GTK allowlist
(`packages/react/src/css-classes-validate.ts`) and rides in the ordinary create/update `props`
JSON; it is not nested under `style` and does not touch the C-ABI vtable (frozen at 18 fields).

- **GTK** applies each class verbatim via `gtk_widget_add_css_class` (`src/gtk/style.zig`'s
  `applyCssClasses`, called from `src/gtk/backend.zig`'s `vtCreate`/`vtApplyProps` whenever
  `props.cssClasses` is present). This is additive-only: a class added on a prior call is never
  removed on update, alongside the internal `nd-<id>` scoping class.
- **macOS** maps the semantic subset onto AppKit control properties (`ndApplyCssClasses` in
  `swift/Sources/NDShell/Backend.swift`), with dynamic system colors throughout so dark mode
  keeps working automatically:

  | Class(es) | AppKit mapping |
  | --- | --- |
  | `suggested-action` | `NSButton.bezelColor = .controlAccentColor`, `keyEquivalent = "\r"` |
  | `destructive-action` | `NSButton.bezelColor = .systemRed`, `hasDestructiveAction = true` |
  | `pill` | no-op — modern AppKit buttons are already rounded |
  | `flat` | `NSButton.isBordered = false`, `showsBorderOnlyWhileMouseInside = true` |
  | `title-1` / `title-2` / `title-3` / `title-4` | `.font = .preferredFont(forTextStyle:)` with `.largeTitle` / `.title1` / `.title2` / `.title3` |
  | `heading` | `.preferredFont(forTextStyle: .headline)` |
  | `caption` / `caption-heading` | `.preferredFont(forTextStyle: .caption1)` / `.caption2` |
  | `body` | `.preferredFont(forTextStyle: .body)` |
  | `dimmed` | `.textColor = .secondaryLabelColor` |
  | `monospace` | `.font = .monospacedSystemFont(ofSize:weight:)` |
  | `numeric` | `.font = .monospacedDigitSystemFont(ofSize:weight:)` |

  The font/color rows target `NSTextField`; for `TextArea`/`ScrollView` widgets (an `NSScrollView`
  wrapping an `NSTextView`) they target the wrapped `NSTextView` instead.

  Structural classes (`navigation-sidebar`, `card`, `view`, `toolbar`, `boxed-list`, `osd`, etc.)
  are silently ignored on macOS; native chrome for those roles comes from the SplitView/HeaderBar
  widgets themselves, not from class strings.

## Adwaita runtime & dark mode

The Linux host runs as `AdwApplication` (`src/gtk/main.zig`), so the Adwaita stylesheet is loaded
and `AdwStyleManager` tracks the system light/dark preference from the first frame. Unstyled
widgets and `cssClasses` follow that preference automatically, with no app code required. Hardcoded
`style` colors are explicit overrides and do not adapt to dark mode; prefer `cssClasses` plus
Adwaita defaults for theme-correct apps.

The window class stays `GtkApplicationWindow`, so the default titlebar and window controls survive
unchanged. A `<headerbar>` (upcoming widget) takes over the titlebar only when an app declares one.
