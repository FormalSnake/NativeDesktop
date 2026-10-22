# Linux look: the libadwaita rules nd follows

On Linux nd draws its controls with GPUI under `Preset::Adwaita`. The target is libadwaita 1.9
(GNOME 49) as it renders, not only as the HIG describes it. Values below come from libadwaita's
compiled stylesheet (`/org/gnome/Adwaita/styles/gtk.css` inside `libadwaita-1.so`) and were checked
against captures of real widgets. The gap tracker is `docs/parity/linux-hig.md`.

Sources: [GNOME HIG](https://developer.gnome.org/hig/),
[libadwaita style classes](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/style-classes.html),
[libadwaita CSS variables](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/css-variables.html),
[libadwaita stylesheet sources](https://gitlab.gnome.org/GNOME/libadwaita/-/tree/main/src/stylesheet).

## Colours

| Role | Light | Dark | nd |
| --- | --- | --- | --- |
| Window | `#fafafb` | `#222226` | `surface.base`, `palette.rs` (`baked_for`, Adwaita arms) |
| View, card | `#ffffff` | `#1d1d20` / card `#343438` | `surface.raised` |
| Sidebar | `#ebebed` | `#2e2e32` | `surface.sidebar` |
| Popover, dialog | `#ffffff`, dialog `#fafafb` | `#36363a` | `surface.overlay` |
| Text | `rgb(0 0 6 / 80%)` | `#ffffff` | `text.primary`; dim is 55% |
| Accent | `#3584e4`, or the GNOME accent setting | same | `accent`; `nd-os-linux/src/hooks.rs` `accent_palettes` |
| Coloured text | OKLab lightness of the bg colour capped at 0.5 (light) or raised to 0.85 (dark) | | `nd_theme::color::standalone`, used for `text.accent/danger/success/warning` |
| Toast, OSD | `#505053`, white text | same | `surface.osd`, `text.on_osd` |
| Dialog dimming | black 15% | black 50% | `scrim` |

GNOME's nine accent names map to `blue #3584e4, teal #2190a4, green #3a944a, yellow #c88800,
orange #ed5b00, red #e62d42, pink #d56199, purple #9141ac, slate #6f8396`, read from
`gsettings org.gnome.desktop.interface accent-color`.

Where libadwaita falls under 4.5:1, nd's appearance audit wins: suggested buttons fill with
`accent_fill` (the accent darkened until white reads at 4.5:1), coloured text stays legible on a
15% tint of itself, and dim text on a selected sidebar row darkens to 4.5:1 on the selection.

## State layers

Fills are the text colour (`currentColor`) at a share, so they work on any surface.

| Control | Rest | Hover | Pressed | nd |
| --- | --- | --- | --- | --- |
| Button | 10% | 15% | 30% | `AdwButton::Regular`, `forms/look.rs` |
| Flat button | 0 | 7% | 16% | `AdwButton::Flat` |
| Toggle button, checked | 30% | 35% | 40% | |
| Suggested | `accent_fill` | + white 10% | + `rgb(0 0 6/20%)` | `AdwButton::Suggested` |
| Destructive | red ink over 15% of it | 20% | 35% | `AdwButton::Destructive` |
| Entry | 10% | | | `FormLook::fill` |
| Switch trough, scale and progress trough | 15% | 20% | 25% | `FormLook::track*` |
| Check and radio ring (2 px inset) | 15% | 20% | 25% fill | `forms/toggle.rs` |
| Menu row | 0 | 10% | 19% | `FormLook::menu_hover/menu_highlight` |
| Navigation sidebar row | 0 | 7% | selected 10%, text unchanged | `nav.rs` `source_tree` |
| Boxed list activatable row | 0 | 4% | 8% | |
| Toggle group: group 10%, toggle hover 7%, pressed 16% | | | | `forms/choice.rs` `segmented` |

## Shape and metrics

- Radii: buttons, entries, rows, menu items 9; check 6; cards and boxed lists 12; popovers 15;
  alert dialogs 18; toggle-group toggles 6 inside a 9 px group with 3 px padding. `preset.rs` radii
  `(6, 9, 12, 15)`.
- Buttons: 34 px high (24 + 5 px padding), text buttons 17 px each side and bold, icon buttons 34x34
  with 16 px symbolic icons, circular 34x34, pill padding 10x32. `render.rs` `button`.
- Entries 34 px, 9 px side padding. Check and radio 20 px (14 + 3 padding). Switch 46x26, knob 20
  (`white 80%` over the view colour, white when hovered or checked, shadow `0 2px 4px rgb(0 0 6/20%)`).
- Scale: 4 px trough, 20 px knob with a 1 px 10% ring and the switch knob shadow. Progress bar 8 px
  trough, 150 px minimum. Spinner 16 px.
- Popover: 1 px `black 14%` border, shadow `0 1px 5px 1px black 9%, 0 2px 14px 3px black 5%`,
  menu box padding 6, rows 32 px with 12 px padding. `FormLook::popover_shadow`, `forms/text.rs`.
- Tooltip: `rgb(0 0 6/80%)`, white text, 1 px white 10% border, padding 6x10, radius 9.
  `overlay.rs`.
- Navigation sidebar rows: 36 px, radius 9, 6 px margin, 2 px apart, icon in the text colour.
- Toast: pill, 46 px, padding 6 (12 leading), bold title, regular action button, flat circular
  close, 12 px from the sides and 24 px from the bottom, shadow `0 1px 3px 1px rgb(0 0 6/7%),
  0 2px 6px 2px rgb(0 0 6/3%)`. `nav.rs` `adw_toast`.
- Banner: `mix(#7d7d83 30%, window)`, 46 px, bold centred title, regular button. nd draws it
  as the text colour at 19% (light) or 12% (dark), which lands within 1 of it and stays a
  token. `nav.rs` `adw_banner`.
- Alert dialog: 372 px wide, heading `title-2` 32 px from the top, body 10 px below it, 24 px
  sides, responses 44 px high (padding 10x20), radius 12, 12 px apart, 24 px bottom.
  `dialogs.rs` `adw_alert`.
- Header bar: 47 px, flat (a toolbar view's top bar draws no shade under it). `drawn.rs` Toolbar.
- Split view: with an open sidebar each pane gets its own header bar, as AdwNavigationSplitView
  and AdwOverlaySplitView do: the sidebar's (sidebar colour, start window controls, the toggle and
  leading items) and the content's (title, the other items, end window controls). `drawn.rs`
  `split_headers`.
- Window controls: 24 px circles at 10/15/30% of the text colour (close included, never red),
  16 px symbolic icons. `nd-os-linux/src/hooks.rs` `window_controls`.

## Type

Adwaita Sans 11 pt. `title-1` 181% weight 800, `title-2` 136%/800, `title-3` 136%/700,
`title-4` 118%/700, `heading` 700, `caption` 82%, dim text 55%. `preset.rs`
`adwaita_text_styles`.

Adwaita Sans ships as one variable file, so every weight is a `wght` instance of the same
face. GPUI's Linux text system shapes and draws a variable face at the requested weight
(`nd-gpui/vendor/gpui-*-wgpu/src/cosmic_text_system.rs` `weighted`); without it bold labels
render at 400.

Links are the standalone accent (`#0461be` light, `#81d0ff` dark), underlined at rest.
`forms/display.rs` `link`.

## Backdrop (inactive window)

libadwaita's `:backdrop` rules, all easing in over 200 ms (CSS `ease-out`): a header bar goes to
`--headerbar-backdrop-color` (the window colour) and its content (`windowhandle`) to 50% opacity, as
do tab bars, search bars, action bars and banners; a sidebar pane goes to
`--sidebar-backdrop-color` (`#f2f2f4` light, `#28282c` dark, halfway between sidebar and window).
Labels, icons and selections outside those bars keep their colours: 1.9 has no `:selected:backdrop`
or `label:backdrop` rule. nd: `Build::inactive` drives the title row's opacity and a group window's
tab bar (`drawn.rs`, `render.rs`), `Theme::sidebar_backdrop` the sidebar (`drawn.rs` Pane).

## Focus

The ring is 2 px of the accent at 50%. Buttons and toggles draw it right outside their edge
(offset 0), switches 1 px out, entries, rows and links 2 px inside. On focus it settles in over
200 ms: it starts 4 px further out and closes onto the control. Buttons are tab stops; a click
does not move keyboard focus. `forms/toggle.rs` `ring`, `focus_outline`; `render.rs` `button`.

## Motion

| What | Duration | Curve | nd |
| --- | --- | --- | --- |
| Hover, press, focus ring | 200 ms | ease-out-quad `cubic-bezier(.25,.46,.45,.94)` | `EASE_OUT_QUAD`, `motion.fast` |
| Revealer, banner | 250 ms | ease-out-cubic | `motion.normal` |
| Toast in | 300 ms | `cubic-bezier(.25,.1,.25,1)`, up from below the window | `adw_toast` |
| Popover open and close | none | | `overlay.rs`, no entrance and no exit ghost |
| Dialog open | spring, damping 0.62, mass 1, stiffness 500, scale 0.8 to 1 | | not yet matched |
| Navigation push | spring, damping 1, mass 1, stiffness 1000 | | `motion.spring` |
| Spinner | 1.2 s per turn, linear | | `FormLook::spinner_seconds` |

Reduced motion (`org.gnome.desktop.interface enable-animations false`) turns transitions into
instant changes; the focus ring then appears at once.

## Interaction

Tab and Shift+Tab move focus, Return activates, Space toggles, Esc closes popovers and dialogs,
F10 opens the primary menu. Dialogs put Cancel first and make Return the default response unless
it is destructive.

## Verifying against libadwaita

1. Capture the references on g815: `scripts/linux-gate run 'scripts/adwaita-reference/capture'`
   (`PAGES=controls APPS=0` narrows it). Boards are built by `scripts/adwaita-reference/board.py`
   and shot by `ndshell shot` in headless Hyprland at scale 2, with a fresh `HOME` so the
   desktop's GTK theme and recent files never leak in.
2. Copy them back: `rsync -a g815:Developer/nativedesktop-<branch>/docs/parity/linux-hig/adwaita/ docs/parity/linux-hig/adwaita/`.
3. Record nd's side: `scripts/linux-gate run 'ND_UPDATE_GOLDENS=1 cargo test -p nd-backend-gpui --test forms --test nav --test layout_widgets'`
   and compare `crates/nd-backend-gpui/tests/goldens/linux/*.png` with the references at the
   same scale, control by control. Sample pixels (Python PIL) for fills and sizes rather than
   judging by eye.
4. The exact rule for any value: `gresource extract <libadwaita-1.so> /org/gnome/Adwaita/styles/gtk.css`
   inside `scripts/adwaita-reference/shell.nix`.
