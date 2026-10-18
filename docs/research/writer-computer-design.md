# writer.computer: design language worth borrowing on Linux

Date: 2026-10-18. Source: https://writer.computer/ and its repo
(github.com/joelbqz/writer-computer, `apps/desktop/src/App.css`,
`apps/desktop/shared/themes/*/*.json`, `apps/desktop/src/lib/theme.ts`).
Not implemented; a reference for M3 (theme, skins) on the GPUI Linux look.

The user likes how this app looks. It is a Tauri app that hand-rolls every
control in HTML, so it is not a model for how nd builds components (nd uses
native controls on macOS and the libadwaita look on Linux). What carries over
is the palette model and a few layout rules.

## Guidelines to adopt

1. A theme is five numbers, not a palette. `accent`, `background`,
   `foreground`, `translucent` (0-100) and `contrast` (0-100). The writer theme
   is `#FF6A00` / `#FFFFFF` / `#0D0D0D` / 10 / 20 light and `#FF6A00` /
   `#111111` / `#FCFCFC` / 20 / 16 dark. Everything else is derived.
2. Every surface except the window background is the foreground at low alpha
   over transparent, never an opaque grey. Borders are `fg * contrast * 24%`,
   hover `* 16%`, selected and active `* 26%`, inputs `* 28%`, focus ring
   `* 65%`, scrollbar thumb `* 58%`. One `contrast` knob moves all of them
   together, and translucency (blur under the window) shows through every layer.
3. The background alpha is applied once, on the root. Nested panes add no
   background layer of their own, since stacking would multiply the opacity.
   This maps onto our see-through sidebar and title row.
4. Text is three tiers of the same foreground: primary 100%, secondary 80%,
   muted 54%, icons 40%.
5. The accent is used sparingly: links and selection (`accent` at 30%).
   Nothing else is tinted.
6. System UI font everywhere, including the editor; mono only for code.
   Reading column capped at 734px, line height 1.875, generous padding that
   scales with window height (`clamp(3rem, 12vh, 9rem)`).
7. Chrome controls are 32px tall with 12px padding; the drag region is 72px.

## Fit with what nd already has

- The five-number model sits naturally on top of the matugen pipeline
  (`linux-nix-config.md` section 1): matugen supplies `accent`, `background`,
  `foreground`; `translucent` and `contrast` become user settings, and the
  alpha derivations replace hand-picked surface colours.
- `FormLook::adwaita` stays: control shapes stay
  libadwaita, their fills and strokes come from these derived tokens.
