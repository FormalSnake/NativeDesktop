# Linux look against libadwaita

Gaps between nd's drawn Linux look (`Preset::Adwaita`, nd-backend-gpui) and libadwaita 1.9, found by
putting nd's goldens next to real libadwaita widgets captured the same way. One line per gap; a tick
names how it was verified. References: `docs/parity/linux-hig/adwaita/<page>-<scheme>.png`, recorded
by `scripts/adwaita-reference/capture` on g815 (headless Hyprland at scale 2, fresh `HOME`, Adwaita
Sans 11). nd's side: the Linux goldens in `crates/nd-backend-gpui/tests/goldens/linux/`.

## Colours and metrics

- [x] Baked Adwaita palette is libadwaita's own (window `#fafafb`/`#222226`, view, sidebar, popover, text at 80%, dim 55%) instead of the generic preset (forms, nav and layout goldens next to the reference pages, colours sampled)
- [x] GNOME accent colour (`org.gnome.desktop.interface accent-color`) drives the accent, with libadwaita's nine named colours (unit test `gnome_accent_names_map_to_libadwaitas_colours`)
- [x] Radii: buttons and entries 9, cards 12, popovers and dialogs 15 (forms and layout goldens next to the controls and dialog references)
- [x] Motion tokens: 200 ms state changes on ease-out-quad, 250 ms reveals (`preset.rs` values; no golden covers timing)

## Controls

- [x] Buttons: 10/15/30% text-colour fills, flat 0/7/16%, bold labels, 34 px icon buttons, 17 px text padding, no border (layout-alert golden next to dialog and controls references)
- [x] Suggested and destructive buttons: accent with a 10% light veil on hover and a 20% shade pressed; destructive as red text on a 15/20/35% red tint (layout-alert golden, `gpui_alert_destructive_paints_danger`)
- [x] Switch: 15/20/25% trough, white knob with libadwaita's knob shadow, accent checked trough with hover and press shades (forms golden next to controls reference)
- [x] Check button: 20 px box at radius 6, 15% inset ring that darkens on hover and fills 25% pressed (forms golden next to controls reference)
- [x] Slider: 4 px trough with hover state, knob with the 10% ring and drop shadow, white under the pointer (forms golden next to controls reference)
- [x] Progress bar: 8 px trough, 150 px minimum; spinner at 16 px over 1.2 s (forms golden next to controls reference)
- [x] Toggle group (segmented): 10% group fill, bold toggles, active toggle white (dark: 20% white) under the toast shadow, 7% hover (forms golden next to controls reference)
- [x] Menus and dropdown popovers: radius 15, 14% black edge and libadwaita's two shadows, 32 px rows, 10% hover and keyboard highlight (forms-menu golden next to the popover reference)
- [x] Overlay popovers open and close at once (GTK 4 has no popover animation); tooltips are 80% near-black with white text, radius 9 (`gpui_a_closed_popover_animates_out_then_is_gone`)
- [x] Focus: buttons are tab stops with a 2 px accent-50% outline right outside; rings on every control settle in over 200 ms; a click leaves keyboard focus alone (forms-menu golden shows the dropdown ring; settling is off in goldens, which run with reduced motion)
- [x] Text weights: bold and heavy text from a variable font (Adwaita Sans) draws at its weight instead of the 400 default instance (forms, nav and layout goldens: bold labels and headings, Fluent body text unchanged)
- [x] Links: standalone accent, underlined at rest (forms golden next to controls reference)
- [x] Coloured text uses libadwaita's standalone colours (destructive `#c30000` light, `#ff938c` dark; accent `#0461be`, `#81d0ff`) (unit test `standalone_colours_match_libadwaitas`, forms golden)

## Navigation and feedback

- [x] Navigation sidebar: neutral selection (10% of the text colour, text unchanged), 7% hover, 36 px rows at radius 9, 2 px apart, 8 px inset, no empty disclosure column (nav-sidebar golden next to app reference)
- [x] Toast: `#505053` pill with white bold title, regular action button and flat close button, 24 px above the window bottom, slides up from below over 300 ms (nav-sidebar golden next to app reference)
- [x] Banner: neutral strip with bold centred title and a regular button, instead of the accent container (nav-sidebar golden next to app reference)
- [x] Alert dialog: radius 18, `title-2` heading, 32/24 px padding, 44 px responses at radius 12, destructive as red tint, dark dimming at 50% (layout-alert goldens next to dialog references)
- [x] Backdrop: an inactive window's header bar content fades to 50% and its sidebar goes to the backdrop colour (halfway to the window's), over 200 ms ease-out; selections and labels elsewhere keep their colours as in libadwaita 1.9 (`gpui_an_inactive_window_takes_the_backdrop_look`, layout-chrome and layout-chrome-backdrop goldens)
- [x] Header bar: 47 px and flat, no line under it, as Nautilus' content header draws (layout-chrome golden next to a Nautilus capture)
- [x] Matugen surfaces: the sidebar and cards take `surface_container`, the colour FormalShell's GTK theme gives Nautilus' sidebar, instead of `surface_container_low` (unit test `formalshell_shape`)
- [ ] Window controls: neutral 10/15/30% circles (close not red) with symbolic minimize, maximize and close icons instead of text glyphs (built and unit-tested; not captured yet: headless goldens draw no window chrome, needs an `ndshell shot` of hello)

## Not matched

- Popover arrow: GTK points menus at their button; nd's popovers have no arrow yet.
- Toast width: AdwToast sizes its title wider than its text; nd sizes the toast to its content.
- Dialog spring (scale 0.8 to 1, damping 0.62) and toast exit (fade with scale 0.95): nd's dialogs and toasts leave at once.
- Kinetic scrolling and overshoot: libadwaita only applies them to touch input, which nd's GPUI backend does not receive on Linux yet.
- Body text is 14 px; Adwaita Sans 11 pt is 14.67 px at 96 dpi. Changing the scale moves every golden of every lane, so it waits for a coordinated pass.
- Contrast: libadwaita's suggested buttons put white on `#3584e4` (3.8:1) and dim text on a selected sidebar row (4.3:1). nd's appearance audit requires 4.5:1, so the suggested fill is `accent_fill` (darkened just enough), dim text on selected rows darkens slightly, and coloured text stays legible on its own 15% tint (dark destructive `#ff9b94` rather than `#ff938c`).
- Split header: Nautilus' sidebar runs the full window height with its own header bar (AdwNavigationSplitView); nd's toolbar spans the window above both panes. Splitting it changes every scaffold app's layout, so it needs its own pass.
- Sidebar section headings: libadwaita uses `.heading` (bold, body size) with 12 px margins; nd keeps its caption-style section label.
- Backdrop sidebar under a matugen palette: nd mixes sidebar and window 50/50, as libadwaita's own palette does; FormalShell's GTK theme sets `sidebar_backdrop_color` to the window colour, so Nautilus goes all the way.
- Dropdown arrow: libadwaita uses the filled `pan-down` triangle; Lucide has no filled triangle, nd keeps its chevron.

