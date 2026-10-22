+++
title = "Theming"
description = "The design tokens apps read with cx.theme(), where their values come from on each OS, and how to change them."
+++

Read colors, spacing, radii, and type sizes from `cx.theme()`, and never write a color literal in
view code. The tokens come from the OS (or a matugen palette on Linux), so a new light or dark
appearance, accent, or OS release reaches your app without a code change. `nd check` flags color
constructors outside a theme file.

<!-- docs:example overrides -->

## Tokens

`cx.theme()` returns the [`Theme`](../../../crates/nd-theme/src/tokens.rs) of the window being
rendered, with that window's override applied. It has these fields:

| Group | Tokens |
| --- | --- |
| Surfaces | `surface.base`, `surface.sidebar`, `surface.raised`, `surface.raised_hover`, `surface.overlay`, `surface.layer` (the opaque content layer beside a Mica sidebar) |
| Lines | `border.subtle`, `border.strong`, `separator` |
| Text | `text.primary`, `text.secondary`, `text.tertiary`, `text.on_accent`, `text.danger` |
| Accent and state | `accent`, `accent_container.{bg, fg}`, `selection`, `danger`, `success`, `warning`, `focus_ring`, `scrim` |
| Metrics | `spacing` (`xs` to `xl`, `content_margin`, `content_width`), `radius` (`sm` to `xl`), `control` heights, `type_scale` (`caption` to `large_title`), `fonts`, `motion`, `materials` |
| Other | `terminal.ansi` (16 colors), `palette` (the base colors that the rest derive from) |

The only inputs are 16 base colors per scheme. The framework computes hover, pressed, disabled,
`focus_ring`, `border.strong`, and `scrim` from them. Use `theme.hover(color)`,
`theme.pressed(color)`, and `theme.disabled(color)` for your own states, and
`theme.text_on(background)` for text on a fill that you chose. The framework nudges `text.primary`
and `text.secondary` until they reach 4.5:1 on the base, sidebar, and raised surfaces, whatever the
source says.

The Chrome example reads spacing, a type size, and a text color:

<!-- docs:snippet examples/chrome/src/lib.rs#content -->

## Presets

A [`Preset`](../../../crates/nd-theme/src/preset.rs) holds the metrics, type, motion, and materials
of one platform. It doesn't include colors. Each backend picks its preset at launch, and apps
override tokens, never the preset.

| Preset | Target | Content margin | Radii (sm to xl) | Body size | Sidebar material |
| --- | --- | --- | --- | --- | --- |
| `MacOs` | macOS | 20 | 4, 6, 10, 16 | 13 | vibrancy |
| `Fluent` | Windows | 24 | 2, 4, 8, 12 | 14 | Mica |
| `Adwaita` | Linux | 12 | 4, 6, 12, 16 | 14 | opaque |
| `Plain` | web | 16 | 4, 6, 8, 12 | 14 | opaque |

When the OS publishes a value (text style sizes, control heights), the backend reads it, and that
value replaces the preset's own. The backend reads the macOS look from AppKit at launch and on every
appearance change instead of copying it into constants. On Windows, the accent comes from
`UISettings` with the shade that WinUI uses for each scheme (AccentDark1 in light, AccentLight2 in
dark), and it follows a change in Settings while the app runs. Grayscale text renders without grid
fitting, the way Windows' own apps draw it on Mica.

On the GPUI backend, text without a size uses the preset's body style (14 px on Fluent and
Adwaita). On the Fluent preset, the last color source, the baked palette, is Fluent's own: WinUI 3's
Mica base, layer, and card fills, its text opacities, and the default accent. A Windows build with
no system accent and the Fluent look drawn on another OS both read as Windows.

`ND_PRESET=fluent` (or `adwaita`, `macos`, `plain`) makes a GPUI build draw another platform's
look. For example, it draws the Fluent skin on Linux, without Mica or caption buttons, so that you
can check it without a Windows machine. That look's palette comes with it (on Linux, a forced preset
reads no matugen file), `on_windows`/`on_linux` follow it, and `appearance = "dark"` in the
`theme.toml` that `ND_THEME_TOML` names picks the scheme. The AppKit backend ignores `ND_PRESET`.

## Where the colors come from

For each of the 16 base colors, the first source that sets it wins:

1. `theme.toml` in the app's config directory.
2. On Linux, the matugen `theme.json`.
3. The OS: appearance, accent, and semantic colors (`NSColor`, `UISettings`, the desktop portal).
4. The baked-in light and dark palettes.

The app, window, and subtree overrides from [Overrides](overrides.md#theme) go on top of the
resolved result. A switch between light and dark, an accent change, or a file edit resolves the
tokens again and redraws once.

On macOS, native controls draw with system colors and ignore the palette unless you override a
token. An overridden accent becomes the control's `bezelColor` or `contentTintColor`. The tokens
matter for the content that you compose yourself, and there they come from the same `NSColor`
sources.

## Appearance

[`Appearance`](../../../crates/nd-theme/src/refinement.rs) is `System`, `Light`, or `Dark`. `System`
inherits from the layer above, so a window set to `Dark` stays dark inside a subtree that says
`System`. To force an appearance for the app, one window, or one subtree, use a `ThemeOverride`:

<!-- docs:snippet examples/overrides/src/lib.rs#window -->

## theme.toml

A `theme.toml` recolors an app without a rebuild, for a user or for the app's own defaults. The
keys are token names, and the values are hex strings. Top-level keys apply to both schemes, and keys
under `[light]` or `[dark]` apply to one.

```toml
appearance = "dark"        # system | light | dark
radius_scale = 1.25
accent = "#3d6bff"

[dark]
"surface.base" = "#101014"
```

The 16 color keys are `surface.base`, `surface.sidebar`, `surface.raised`,
`surface.overlay`, `text.primary`, `text.secondary`, `text.tertiary`, `accent`, `text.on_accent`,
`accent.container`, `accent.on_container`, `separator`, `selection`, `danger`, `success`, and
`warning`. An unknown key, a value that isn't hex, or an unknown `appearance` rejects the whole
file, so you notice a typo instead of having it ignored. After a rejected edit, the app keeps the
last good theme.

The file is read from `<config>/<app>/theme.toml`: `~/Library/Application Support` on macOS,
`$XDG_CONFIG_HOME` (default `~/.config`) on Linux, and `%APPDATA%` on Windows.
`ND_THEME_TOML=<path>` points somewhere else, and an empty value turns the source off. See
[settings and config](../guides/settings-and-config.md) for the rest of the app's config files.

## Matugen on Linux

Your app never runs matugen. When the desktop runs matugen, it renders nd's template,
[`theme.json`](../../../crates/nd-theme/templates/matugen/theme.json), to
`~/.config/<app>/theme.json`, and the app watches that file. Register the template in matugen's
`config.toml` with `input_path` pointing at the template and `output_path` at
`~/.config/<app>/theme.json`. The Material You roles map to tokens as `surface` to
`surface.base`, `surface_container` to `surface.sidebar` and `surface.raised` (where the desktop's
GTK colours put sidebars and cards), `primary` to `accent`, `error` to
`danger`, and so on (the full table is `ROLE_MAP` in
[matugen.rs](../../../crates/nd-theme/src/matugen.rs)). Material has no success or warning role,
so those two come from the `green` and `yellow` custom colors.

A change to the file reaches the app within about 70 ms. The watcher listens on the parent
directory (atomic writers replace the file), reads the first event at once, reads again 50 ms later,
and polls the modification time once a second as a fallback. The app skips a half-written or invalid
file and keeps the last palette. On other OSes, you opt in to the file through `ND_THEME_JSON`.

## The appearance audit

Every app ships a `tests/appearance.rs` that renders its windows in light, in dark, and, on the
GPUI backend, in two matugen themes. It reads back every text run and fill that the backend painted.
It fails when a color isn't a token (or a state derived from one) or when text falls below 4.5:1
contrast (3:1 for large text).

<!-- docs:snippet examples/overrides/tests/appearance.rs -->

`allow` lists colors that the app names on purpose in its theme file, such as a brand fill. The
audit still checks their contrast. On macOS, the audit runs once on AppKit and once on GPUI, each in
its own process. Run it with `cargo test -p overrides --test appearance`; the
[testing page](../tooling/testing.md) covers the other tiers.
