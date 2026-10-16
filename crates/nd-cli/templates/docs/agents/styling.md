# Styling

Colours, sizes and type come from the theme. The OS supplies most of the theme, so an app that
reads tokens follows light and dark, the user's accent, matugen on Linux and the next OS
redesign without a change. Rules in short: [donts.md](donts.md).

## Read tokens, never literals

`cx.theme()` returns the tokens for the window being rendered:

| Group | Tokens |
| --- | --- |
| Surfaces | `surface.base`, `surface.sidebar`, `surface.raised`, `surface.raised_hover`, `surface.overlay` |
| Text | `text.primary`, `text.secondary`, `text.tertiary`, `text.on_accent`, `text.danger` |
| Lines | `border.subtle`, `border.strong`, `separator` |
| Colour roles | `accent`, `accent_container.{bg, fg}`, `selection`, `danger`, `success`, `warning`, `focus_ring`, `scrim` |
| Spacing | `spacing.{xs, sm, md, lg, xl}`, `spacing.content_margin`, `spacing.content_width` |
| Shape and size | `radius.{sm, md, lg, xl}`, `control.{small, regular, large}` |
| Type | `type_scale.{caption, footnote, body, headline, title, large_title}`, `fonts.{body, rounded, mono}` |
| Motion | `motion.{fast, normal, slow}`, `motion.spring` |

Do:

```rust
let theme = cx.theme();
v_flex()
    .p(theme.spacing.content_margin)
    .gap(theme.spacing.sm)
    .child(text(title).text_size(theme.type_scale.title))
    .child(text(subtitle).text_color(theme.text.secondary))
```

Don't:

```rust
v_flex()
    .p(px(20.))
    .child(text(title).text_size(px(22.)).text_color(rgb(0x1c1c1e)))
    .child(text(subtitle).text_color(rgb(0x8e8e93)))
```

The second one is right in one appearance on one OS. In dark mode the text vanishes, under
matugen it clashes, and the next macOS changes the system grey while this stays put. `nd check`
flags the colour constructors (`colour-literal`); the appearance audit catches the colour
whatever produced it.

On macOS the token values themselves are read from the OS (`NSColor` semantic colours, text
styles, control sizes), so even composed content matches the system. A `px(..)` is fine for a
size that belongs to the content (an avatar, a sidebar's minimum width); spacing, radii, control
heights and type sizes come from tokens.

## Where colours may live

The one place an app names colours is its theme file: `src/theme.rs` or anything under
`src/theme/`. `colour-literal` allows constructors there and nowhere else.

```rust
// src/theme.rs
pub fn app() -> ThemeOverride {
    ThemeOverride::new().accent(rgb(0x3d6bff)).radius_scale(1.25)
}

pub fn brand_colors() -> [Hsla; 1] {
    [rgb(0x3d6bff).into()]
}
```

A colour named there on purpose (a brand tint, a tag colour) is passed to the audit with
`.allow(theme::brand_colors())`; anything else that is not a token fails it.

## The appearance audit

`tests/appearance.rs` runs in `nd check` and `nd test`. It renders every window it lists in light
and dark on each backend kind the target links (and two matugen palettes on GPUI), reads back
every painted fill and text run, and fails when:

- a colour is not a theme token or a state derived from one (hover, pressed, disabled);
- text is under WCAG contrast against what it sits on: 4.5:1, or 3:1 for large text (24 px, or
  18.66 px bold).

Add a `.window(..)` for every window the app opens. A window the audit never sees is a window
whose dark mode nobody checked.

## Overrides, from narrowest to widest

Defaults are native. Each layer below opts one thing out; use the narrowest layer that does the
job.

| Layer | How | Right when |
| --- | --- | --- |
| Element style | `.bg(..)`, `.text_color(..)`, `.font(..)`, `.rounded_*()` on a control | one control needs a tint or font. Native controls take what they can express (accent as bezel colour, fonts); what they cannot is ignored with a dev warning |
| Per platform | `.on_macos(\|b\| b.bezel(Bezel::Glass))`, `.on_windows(..)`, `.on_linux(..)`, `.on_web(..)` | one OS deserves a different variant. The closure runs only there, so no `cfg` |
| Mapping | `.native(Mapping::Segmented)` on a `select`; `.drawn()` | another native control fits better, or the look cannot be expressed natively. `.drawn()` needs a reason comment (`drawn-reason`) |
| Subtree theme | `.theme(\|t\| t.accent(..).radius_scale(1.5))` on a container | one region (a card, a sidebar section) has its own accent |
| Window theme | `window.set_theme(ThemeOverride::new().appearance(Appearance::Dark), cx)` | one window is always dark, or has its own accent |
| App theme | `Application::new(..).theme(theme::app())`, or `nd::theme::set_app_theme(cx, ..)` at runtime | brand accent and radius scale for the whole app |
| Patterns | `.pattern(Pattern::Drawn)` on a container, `Application::patterns(Patterns { .. })`, `[patterns]` in `nativedesktop.toml` | menus, tooltips or popovers drawn by nd instead of the OS |
| Window chrome | `WindowOptions::main(..).titlebar(Titlebar::Custom)` with `titlebar()` | content runs under the titlebar, with the OS controls in their reserved inset |

Never reach for `.drawn()` on a structural element (sidebar, toolbar, window controls) to get a
look. That is the replica the rule against replicas is about.

## Glass and materials

`glass()` is a container on the system material: `NSGlassEffectView` on macOS 26 and later,
`NSVisualEffectView` below, Mica or Acrylic on Windows, compositor blur on Linux where the
compositor offers it.

```rust
glass().variant(GlassVariant::Clear).rounded_lg().p_3().child(text("Drafts sync when you are online."))
```

Shape follows Apple's HIG: glass the size of a control (64 pt or less on its short side) is a
capsule, or a circle when it is square, unless the app gives it a radius (HIG, Buttons: "In
general, prefer circular or capsule-shape buttons"); larger glass is a surface and stays square
unless rounded; a custom component in a bar keeps its corner radius concentric with the bar's
(HIG, Toolbars). Round buttons are `Bezel::Circle`, the system's own glass circle on 26.

`.tint(color)` takes a colour from the theme file, `glass_group()` merges nearby glass where the
OS does that. Never fake glass with a translucent fill and a blur of your own: it does not pick
up the wallpaper, the window's activity or the user's reduced-transparency setting.

## Palettes on Linux

On Linux the palette comes from matugen's `~/.config/<app>/theme.json`, watched and applied live
(a half-written file keeps the last palette). A `theme.toml` in the app's config directory
overrides tokens on every OS. Neither needs app code; both are why literals are banned.
