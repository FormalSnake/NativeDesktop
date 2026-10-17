+++
title = "Overrides"
description = "Change how a native default looks or behaves, from one element's style up to the whole app, and pick the right layer for each change."
+++

Every component starts with the OS's own default. Start with no overrides, and add one only when
you want a specific difference. Each layer below changes less than the one after it, so pick the
smallest layer that does the job. The Overrides example uses every layer in one window.

<!-- docs:example overrides -->

| You want | Layer | Reach for |
| --- | --- | --- |
| One element tinted, rounded, or in another font | [element style](#element-style) | `.bg()`, `.rounded()`, `.font()` |
| One element different on one OS | [per platform](#per-platform) | `.on_macos(..)`, `.on_linux(..)` |
| A different native control, or a drawn one | [mapping](#mapping) | `.native(Mapping::..)`, `.drawn()` |
| New colors, accent, or radii | [theme](#theme) | `Application::theme`, `window.set_theme`, `.theme(..)` |
| A drawn menu, tooltip or titlebar instead of the OS's | [patterns](#patterns) | `.pattern(..)`, `Application::patterns`, `[patterns]` |

## Element style

Style methods apply to a native control when the control can express them. On a macOS button, a
fill becomes `bezelColor`, a text color becomes `contentTintColor`, and a font becomes an `NSFont`.

<!-- docs:snippet examples/overrides/src/lib.rs#element-style -->

When the native control can't draw a property, such as a corner radius on an `NSButton`, nd
ignores it, and a dev build prints one warning that names the fix. When you see that warning, move
to [mapping](#mapping) instead of stacking more style on top. Take the color from a token or a
theme file. A literal color in view code fails `nd check`.

## Per platform

`.on_macos`, `.on_linux`, `.on_windows`, and `.on_web` take a closure that runs only on that
platform, so the call site needs no `cfg`. Methods that mean something on only one OS, such as
`bezel`, exist on every target, and the other targets ignore them.

<!-- docs:snippet examples/overrides/src/lib.rs#per-platform -->

Use it when the OSes offer different things: a glass bezel exists on macOS 26, and an accent
button style exists on Windows. Don't use it to give every platform the same pixels, because that
undoes the native look that the framework exists to keep. When two branches differ only in color
or radius, put the difference in the theme instead.

## Mapping

A mapping override changes which implementation draws an element. `.drawn()` forces the composed
implementation on every OS, and that implementation accepts every style override that the native
control can't. Today `slider` has it. `.native(Mapping::..)` picks another native control for the
same element: `select` can be a pop-up, a segmented control, or a radio group.

<!-- docs:snippet examples/overrides/src/lib.rs#mapping -->

A drawn control looks the same everywhere and doesn't follow OS restyling. Use `.drawn()` when the
exact look matters most, and use `.native(..)` when the OS has a control that fits.

## Theme

Theme overrides change tokens (see [Theming](theming.md#tokens)). They stack, and a deeper layer
wins over a shallower one:

- App: `Application::theme(..)`, on top of the platform preset and palette sources.
- Window: `window.set_theme(..)` inside `open_window`, or `window.clear_theme(cx)`.
- Subtree: `.theme(|t| t.accent(..))` on any container; everything inside resolves against it.

<!-- docs:snippet examples/overrides/src/lib.rs#app -->

The app layer is in [theme.rs](../../../examples/overrides/src/theme.rs), the only file in the
example that names colors:

<!-- docs:snippet examples/overrides/src/theme.rs -->

The subtree layer wraps a tinted glass container, so its accent button and radii differ from the
rest of the window:

<!-- docs:snippet examples/overrides/src/lib.rs#subtree -->

`radius_scale` multiplies down the layers: an app scale of 1.25 and a subtree scale of 1.5 give
1.875. On macOS, the accent reaches native controls inside the subtree as their bezel color, and a
forced appearance sets the subtree's `NSAppearance`. To recolor the whole app in a way that users
can edit, ship a `theme.toml` instead of code (see [Theming](theming.md#themetoml)).

## Patterns

A pattern is a structure that the OS draws itself, such as a menu, tooltip, popover, or titlebar.
A pattern override swaps the OS's version for one that nd draws. You can set it in these places,
from narrowest to widest:

- `.pattern(Pattern::Drawn)` on one container. It covers the menus and tooltips inside it.
- `Application::patterns(Patterns { tooltips: Pattern::Drawn, .. })` for the app.
- `[patterns]` in `nativedesktop.toml`, below the code: `tooltips`, `menus`, `popovers`,
  `context_menu` (each `native` or `drawn`), and `settings_window` (`window` or `sheet`).
- `WindowOptions::titlebar(Titlebar::Custom)` for a custom titlebar on macOS too.

Build a custom titlebar with `titlebar()`. It reserves the room that the OS controls need, so you
don't tune any margin by hand:

<!-- docs:snippet examples/overrides/src/lib.rs#titlebar -->

On macOS, the traffic lights sit on the left. On Windows, the caption buttons sit on the right. On
Linux, GNOME's `button-layout` decides, and Hyprland has no controls. See [Patterns](patterns.md)
for what each structure maps to.

## When not to override

- Don't hand-tune padding, magic offsets, or one-off colors to make one screen look native. If a
  structure looks wrong on an OS, fix it in the backend so that every app gets the fix.
- Don't use `cfg(target_os)` in view code. `.on_*` keeps both branches type-checked on every
  target.
- Don't use `.drawn()` or a drawn pattern to avoid learning what the native control offers. A
  drawn control stops following the OS when the OS restyles.

To run the example's tests, use `cargo test -p overrides`. The appearance audit checks that its
colors are tokens (see [Theming](theming.md#the-appearance-audit)). The macOS goldens compare the
showcase in light and dark mode, and the test process needs Screen Recording permission.
