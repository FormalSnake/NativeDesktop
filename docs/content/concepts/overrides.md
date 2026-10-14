+++
title = "Overrides"
description = "Every way to change how a native default looks or behaves, from one element's style up to the whole app, and when each is the right one."
+++

The default for every component is the OS's own, so start with no overrides and add one only for
a concrete difference you want. Each layer below changes less than the one after it: pick the
smallest that does the job. The Overrides example uses every layer in one window.

<!-- docs:example overrides -->

| You want | Layer | Reach for |
| --- | --- | --- |
| One element tinted, rounded or in another font | [element style](#element-style) | `.bg()`, `.rounded()`, `.font()` |
| One element different on one OS | [per platform](#per-platform) | `.on_macos(..)`, `.on_linux(..)` |
| A different native control, or a drawn one | [mapping](#mapping) | `.native(Mapping::..)`, `.drawn()` |
| New colours, accent or radii | [theme](#theme) | `Application::theme`, `window.set_theme`, `.theme(..)` |
| A drawn menu, tooltip or titlebar instead of the OS's | [patterns](#patterns) | `.pattern(..)`, `Application::patterns`, `[patterns]` |

## Element style

Style methods apply to native controls where the control can express them: a fill becomes
`bezelColor` on a macOS button, a text colour `contentTintColor`, a font an `NSFont`.

<!-- docs:snippet examples/overrides/src/lib.rs#element-style -->

A property the native control cannot draw, such as a corner radius on an `NSButton`, is ignored,
and a dev build prints one warning that names the fix. Take that warning as the signal to move to
[mapping](#mapping), not to stack more style on top. Use the colour from a token or a theme file;
a literal in view code fails `nd check`.

## Per platform

`.on_macos`, `.on_linux`, `.on_windows` and `.on_web` take a closure that runs only on that
platform, so the call site needs no `cfg`. Methods that only mean something on one OS, such as
`bezel`, exist on every target and are ignored elsewhere.

<!-- docs:snippet examples/overrides/src/lib.rs#per-platform -->

Use it for a real difference in what the OS offers (a glass bezel exists on macOS 26, an accent
button style on Windows). Do not use it to give every platform the same pixels; that undoes the
native look the framework exists to keep. When two branches differ only in colour or radius, the
theme is the better place.

## Mapping

A mapping override changes which implementation draws an element. `.drawn()` forces the composed
implementation on every OS, which takes every style override the native control cannot. Today
it is on `slider`. `.native(Mapping::..)` picks another native control for the same element:
`select` can be a pop-up, a segmented control or a radio group.

<!-- docs:snippet examples/overrides/src/lib.rs#mapping -->

A drawn control looks the same everywhere and does not follow OS restyling, so use `.drawn()` when
the look is the point. Pick `.native(..)` when the OS has a control that fits.

## Theme

Theme overrides change tokens (see [Theming](theming.md#tokens)). They stack, and a deeper layer
wins over a shallower one:

- App: `Application::theme(..)`, on top of the platform preset and palette sources.
- Window: `window.set_theme(..)` inside `open_window`, or `window.clear_theme(cx)`.
- Subtree: `.theme(|t| t.accent(..))` on any container; everything inside resolves against it.

<!-- docs:snippet examples/overrides/src/lib.rs#app -->

The app layer lives in [theme.rs](../../../examples/overrides/src/theme.rs), the only file in the
example that names colours:

<!-- docs:snippet examples/overrides/src/theme.rs -->

The subtree layer wraps a tinted glass container, so its accent button and radii differ from the
rest of the window:

<!-- docs:snippet examples/overrides/src/lib.rs#subtree -->

`radius_scale` multiplies down the layers: an app scale of 1.25 and a subtree scale of 1.5 give
1.875. On macOS the accent reaches native controls inside the subtree as their bezel colour, and a
forced appearance sets the subtree's `NSAppearance`. For a whole-app recolour that users can edit,
ship a `theme.toml` instead of code (see [Theming](theming.md#themetoml)).

## Patterns

A pattern is a structure the OS draws itself: a menu, a tooltip, a popover, a titlebar. A pattern
override swaps the OS's version for one nd draws. Set it, from narrowest to widest:

- `.pattern(Pattern::Drawn)` on one container, which covers the menus and tooltips inside it.
- `Application::patterns(Patterns { tooltips: Pattern::Drawn, .. })` for the app.
- `[patterns]` in `nativedesktop.toml`, below the code: `tooltips`, `menus`, `popovers`,
  `context_menu` (each `native` or `drawn`) and `settings_window` (`window` or `sheet`).
- `WindowOptions::titlebar(Titlebar::Custom)` for a custom titlebar on macOS too.

A custom titlebar is built with `titlebar()`. It reserves the room the OS controls need, so no
margin is tuned by hand:

<!-- docs:snippet examples/overrides/src/lib.rs#titlebar -->

Traffic lights sit on the left on macOS, caption buttons on the right on Windows, GNOME's
`button-layout` decides on Linux, and Hyprland has none. See [Patterns](patterns.md) for what each
structure maps to.

## When not to override

- Do not hand-tune padding, magic offsets or one-off colours to make one screen look native. If a
  structure looks wrong on an OS, the fix belongs in the backend so every app gets it.
- Do not use `cfg(target_os)` in view code. `.on_*` keeps both branches type-checked on every
  target.
- Do not reach for `.drawn()` or a drawn pattern to avoid learning what the native control offers.
  A drawn control stops following the OS when the OS restyles.

Run the example's tests with `cargo test -p overrides`. The appearance audit checks that its
colours are tokens (see [Theming](theming.md#the-appearance-audit)); the macOS goldens compare the
showcase in light and dark and need Screen Recording permission for the test process.
