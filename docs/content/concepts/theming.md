+++
title = "Theming"
description = "The design tokens apps read with cx.theme(), where their values come from on each OS, and how to change them."
+++

Read colours, spacing, radii and type sizes from `cx.theme()` and never write a colour literal in
view code. The tokens come from the OS (or a matugen palette on Linux), so a new light or dark
appearance, accent or OS release reaches the app without a code change. `nd check` flags colour
constructors outside a theme file.

<!-- docs:example overrides -->

## Tokens

`cx.theme()` returns the [`Theme`](../../../crates/nd-theme/src/tokens.rs) of the window being
rendered, with that window's override applied. The fields:

| Group | Tokens |
| --- | --- |
| Surfaces | `surface.base`, `surface.sidebar`, `surface.raised`, `surface.raised_hover`, `surface.overlay`, `surface.layer` (the opaque content layer beside a Mica sidebar) |
| Lines | `border.subtle`, `border.strong`, `separator` |
| Text | `text.primary`, `text.secondary`, `text.tertiary`, `text.on_accent`, `text.danger` |
| Accent and state | `accent`, `accent_container.{bg, fg}`, `selection`, `danger`, `success`, `warning`, `focus_ring`, `scrim` |
| Metrics | `spacing` (`xs` to `xl`, `content_margin`, `content_width`), `radius` (`sm` to `xl`), `control` heights, `type_scale` (`caption` to `large_title`), `fonts`, `motion`, `materials` |
| Other | `terminal.ansi` (16 colours), `palette` (the base colours the rest derive from) |

Sixteen base colours per scheme are the only inputs; hover, pressed, disabled, `focus_ring`,
`border.strong` and `scrim` are computed from them. Use `theme.hover(color)`, `theme.pressed(color)`
and `theme.disabled(color)` for your own states, and `theme.text_on(background)` for text on a
fill you chose. `text.primary` and `text.secondary` are nudged until they reach 4.5:1 on the
base, sidebar and raised surfaces, whatever the source says.

The Chrome example reads spacing, a type size and a text colour:

<!-- docs:snippet examples/chrome/src/lib.rs#content -->

## Presets

A [`Preset`](../../../crates/nd-theme/src/preset.rs) holds the metrics, type, motion and materials
of one platform. Colours are not part of it. Each backend picks its preset at launch and apps
override tokens, never the preset.

| Preset | Target | Content margin | Radii (sm to xl) | Body size | Sidebar material |
| --- | --- | --- | --- | --- | --- |
| `MacOs` | macOS | 20 | 4, 6, 10, 16 | 13 | vibrancy |
| `Fluent` | Windows | 24 | 2, 4, 8, 12 | 14 | Mica |
| `Adwaita` | Linux | 12 | 4, 6, 12, 16 | 14 | opaque |
| `Plain` | web | 16 | 4, 6, 8, 12 | 14 | opaque |

Where the OS publishes a value (text style sizes, control heights), the backend reads it and it
replaces the preset's own. The macOS look is read from AppKit at launch and on every appearance
change, not copied into constants. On Windows the accent comes from `UISettings` with the shade
WinUI uses for each scheme (AccentDark1 in light, AccentLight2 in dark) and follows a change in
Settings while the app runs; grayscale text is rendered without grid fitting, as Windows' own apps
draw it on Mica.

## Where the colours come from

For each of the 16 base colours the first source that sets it wins:

1. `theme.toml` in the app's config directory.
2. On Linux, the matugen `theme.json`.
3. The OS: appearance, accent and semantic colours (`NSColor`, `UISettings`, the desktop portal).
4. The baked-in light and dark palettes.

The app, window and subtree overrides from [Overrides](overrides.md#theme) go on top of the
resolved result. A light/dark flip, accent change or file edit re-resolves the tokens and redraws
once.

On macOS, native controls draw with system colours and ignore the palette unless a token is
overridden: an overridden accent becomes the control's `bezelColor` or `contentTintColor`. The
tokens matter for the content you compose yourself, and there they come from the same `NSColor`
sources.

## Appearance

[`Appearance`](../../../crates/nd-theme/src/refinement.rs) is `System`, `Light` or `Dark`. `System`
inherits from the layer above, so a window set to `Dark` stays dark inside a subtree that says
`System`. Force an appearance for the app, one window or one subtree with a `ThemeOverride`:

<!-- docs:snippet examples/overrides/src/lib.rs#window -->

## theme.toml

A `theme.toml` recolours an app without a rebuild, for a user or for the app's own defaults.
Token names are the keys, hex strings the values. Top-level keys apply to both schemes, and
`[light]` and `[dark]` apply to one.

```toml
appearance = "dark"        # system | light | dark
radius_scale = 1.25
accent = "#3d6bff"

[dark]
"surface.base" = "#101014"
```

The sixteen colour keys are `surface.base`, `surface.sidebar`, `surface.raised`,
`surface.overlay`, `text.primary`, `text.secondary`, `text.tertiary`, `accent`, `text.on_accent`,
`accent.container`, `accent.on_container`, `separator`, `selection`, `danger`, `success` and
`warning`. An unknown key, a value that is not hex or an unknown `appearance` rejects the whole
file, so a typo is noticed instead of ignored. A rejected edit keeps the last good theme.

The file is read from `<config>/<app>/theme.toml`: `~/Library/Application Support` on macOS,
`$XDG_CONFIG_HOME` (default `~/.config`) on Linux, `%APPDATA%` on Windows. `ND_THEME_TOML=<path>`
points elsewhere and an empty value turns the source off. See
[settings and config](../guides/settings-and-config.md) for the rest of the app's config files.

## Matugen on Linux

The app never runs matugen. The desktop's matugen run renders nd's template,
[`theme.json`](../../../crates/nd-theme/templates/matugen/theme.json), to
`~/.config/<app>/theme.json`, and the app watches that file. Register the template in matugen's
`config.toml` with `input_path` pointing at the template and `output_path` at
`~/.config/<app>/theme.json`. The Material You roles map to tokens as `surface` to
`surface.base`, `surface_container_low` to `surface.sidebar`, `primary` to `accent`, `error` to
`danger`, and so on (the full table is `ROLE_MAP` in
[matugen.rs](../../../crates/nd-theme/src/matugen.rs)). Material has no success or warning role,
so those two come from the `green` and `yellow` custom colours.

A change to the file lands in the app within about 70 ms. The watcher listens on the parent
directory (atomic writers replace the file), reads the first event at once, rereads 50 ms later,
and polls the modification time once a second as a fallback. A half-written or invalid file is
skipped and the last palette stays. On other OSes the file is opt-in through `ND_THEME_JSON`.

## The appearance audit

Every app ships a `tests/appearance.rs` that renders its windows in light, dark and, on the GPUI
backend, two matugen themes. It reads back every text run and fill the backend painted and fails
when a colour is not a token (or a state derived from one) or when text falls under 4.5:1
contrast (3:1 for large text).

<!-- docs:snippet examples/overrides/tests/appearance.rs -->

`allow` lists colours the app names on purpose in its theme file, such as a brand fill. Contrast
is still checked for them. On macOS the audit runs once on AppKit and once on GPUI, each in its
own process. Run it with `cargo test -p overrides --test appearance`; the
[testing page](../tooling/testing.md) covers the other tiers.
