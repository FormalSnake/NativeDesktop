# Windows: the WinUI 3 rules nd draws by

nd draws its own Fluent skin with GPUI on Windows (no XAML), under `Preset::Fluent` (the default
there, `ND_PRESET=fluent` elsewhere for goldens). The target is a control that is hard to tell
apart from the real WinUI 3 one in the WinUI 3 Gallery and the Settings app at the same scale.
The gap tracker is [docs/parity/windows-hig.md](../../parity/windows-hig.md).

Sources, read in this order when a value is missing here:

- Theme resources: `https://github.com/microsoft/microsoft-ui-xaml/tree/main/controls/dev`, files
  `CommonStyles/<Control>_themeresources.xaml`, `CommonStyles/Common_themeresources_any.xaml`
  (colours) and `<Control>/<Control>_themeresources.xaml` for the newer controls.
- Design guidance: `https://learn.microsoft.com/windows/apps/design/` (typography, spacing,
  motion, color, geometry).
- Measurement: captures of the WinUI 3 Gallery (store id `9P3JFPWWDZRC`) in the g815 VM. A
  measured pixel wins over a guideline when the two disagree.

## Type

Segoe UI Variable Text, the WinUI type ramp (size/weight/line height): Caption 12/400/16, Body
14/400/20, Body Strong 14/600/20, Body Large 18/400/24, Subtitle 20/600/28, Title 28/600/36,
Title Large 40/600/52. Source: learn.microsoft.com, Typography in Windows. nd:
`crates/nd-theme/src/preset.rs` (`Preset::Fluent`, `text_styles`, `type_scale`). A form's title
is Title (28 semibold, `drawn.rs`). Text on Mica renders grayscale without grid fitting (fork
patch 0007).

## Geometry

- ControlCornerRadius 4, OverlayCornerRadius 8 (flyouts, dialogs). nd: `radius.md` and `radius.lg`.
- Control height 32 (Button, TextBox, ComboBox, NumberBox), CheckBox and RadioButton rows at
  least 32 (`FormLook::toggle_row`), RadioButtons row spacing 8, column spacing 7
  (`radio_spacing`).
- Button padding 11,5,11,6; TextBox padding 10,5,6,6; label 8 px from a check box or radio glyph.
- NavigationViewItem: 36 px rows, 4 px from the pane edge and 4 px apart, icon 12 px in, label
  16 px past the 16 px icon, children indented 32, expand chevron (E70D/E70E, 12 px) at the
  trailing edge 14 px in, headers in Body Strong secondary. nd: `nav.rs` (`source_tree_view`,
  `FLUENT_ROW`).
- InfoBar: min height 48, padding 16 left, icon 16 with 14 to the text, action 16 from the text,
  radius 4, 1 px CardStrokeColorDefault. nd: `nav.rs` (`info_bar`).
- NumberBox: min width 120, inline up and down RepeatButtons 32 wide, 4 px margin, chevrons
  E70E/E70D at 12 px. nd: `forms/value.rs` (`number_box`).
- ComboBox drop-down: opens over the button with the current item on it, as wide as the button,
  padding 5 across and 6 along, items 32 tall 4 apart. nd: `FormLook::{menu_over, popover_pad,
  item_gap}`, `forms/choice.rs`.

- MenuFlyout: presenter padding 0,2, items 32 px with margin 4,2 and padding 11 (the narrow
  mouse padding), radius 4, a 28 px check column, separators 1 px full width. Overlay radius 8,
  SurfaceStrokeColorFlyout, the flyout shadow. nd: `overlay.rs` (`menu`); popovers and tooltips
  share the stroke and shadow.

## Colour (light / dark)

From `Common_themeresources_any.xaml`. nd: `FormLook::fluent` in
`crates/nd-backend-gpui/src/forms/look.rs` unless noted.

| Resource | Light | Dark | nd |
| --- | --- | --- | --- |
| TextFillColorPrimary / Secondary | `#E4000000` / `#9E000000` | `#FFFFFF` / `#C5FFFFFF` | theme `text` |
| ControlFillColorDefault / Secondary / Tertiary | `#B3FFFFFF` / `#80F9F9F9` / `#4DF9F9F9` | `#0FFFFFFF` / `#15FFFFFF` / `#08FFFFFF` | `fill`, `fill_hover`, `fill_active` |
| ControlFillColorInputActive | `#FFFFFF` | `#B31E1E1E` | `field_focus_fill` |
| ControlStrokeColorDefault | `#0F000000` | `#12FFFFFF` | `border`, `field_stroke` |
| ControlStrokeColorSecondary (bottom edge) | `#29000000` | `#18FFFFFF` | `stroke_bottom` |
| ControlStrongFill / StrongStroke | `#72000000` | `#8BFFFFFF` | `track`, `switch_outline`, `check_stroke`, `field_underline` |
| ControlAltFillColorSecondary / Tertiary / Quarternary | `#06000000` / `#0F000000` / `#18000000` | `#19000000` / `#0BFFFFFF` / `#12FFFFFF` | `check_fill` (off switch, check box, radio) |
| SubtleFillColorSecondary / Tertiary | `#09000000` / `#06000000` | `#0FFFFFFF` / `#0AFFFFFF` | `subtle_hover`, `subtle_active` |
| SurfaceStrokeColorFlyout | `#0F000000` | `#33000000` | `popover_stroke` |
| SystemFillColorAttentionBackground | `#80F6F6F6` | `#08FFFFFF` | InfoBar fill |
| CardStrokeColorDefault | `#0F000000` | `#19000000` | InfoBar stroke |
| Accent fill | SystemAccentColorDark1 | SystemAccentColorLight2 | theme `accent` (`nd-os-windows/src/colors.rs` reads UISettings) |

Accent hover is the accent at 0.9 opacity, pressed at 0.8 (`accent_hover`, `accent_active`).
Disabled controls take 0.4 opacity. Layers: Mica window, opaque content layer `#f9f9f9` light
(Notepad) and `#191919` dark (Task Manager) with the window's corner radius (`surface.layer`).

## States and elevation

- Buttons and fields carry a 1 px ControlStroke border whose bottom edge is darker
  (ControlStrokeColorSecondary; on accent buttons ControlStrokeColorOnAccentSecondary), drawn as
  an overlay (`forms/toggle.rs` `elevation_edge`).
- TextBox: bottom edge ControlStrongStroke, focused 2 px accent underline, fill InputActive.
- CheckBox 20 px, glyph 12 (E73E check, E9AE indeterminate); RadioButton dot 12, 14 hovered,
  10 pressed; ToggleSwitch 40x20, knob 12, 14 hovered, 17x14 pressed; Slider thumb 18 plus 1 px
  border with a 12 px accent dot (14 hovered, 10 pressed), track 4 radius 2, ticks 4.
- ProgressBar: 1 px rail, 3 px bar. ProgressRing: accent, no track, 32 px, 4 px stroke.
- Selection: a 3x16 accent pill radius 1.5 on the leading edge of the selected list, menu or
  nav item over a subtle fill, never a check mark (`forms/text.rs` `selection_pill`).
- HyperlinkButton: accent text, button padding, subtle fill under the pointer, no underline
  (`FormLook::link_button`).
- The pointer stays an arrow over controls; only links take the hand (`FormLook::cursor`).
- Text selection: AccentFillColorSelectedTextBackground (solid accent) under white text
  (`FormLook::{selection, selection_text}`). A TextBox header is Body, regular, 8 px above the
  control (`form_field`).

## Inactive window

WinUI's TitleBar (`TitleBar.xaml`, `TitleBar_themeresources.xaml`): while the window is
deactivated the title and the back and pane buttons turn TextFillColorTertiary, and the icon, the
headers and the content sit at `TitleBarDeactivatedOpacity` 0.5, switched at once. nd draws its
title row's content at half opacity (primary text at 50% lands within a step of tertiary) and
leaves the navigation pane as it is; Mica's inactive fallback is the OS's. `drawn.rs` (Toolbar),
`Build::inactive`.

## Focus visual

Two rings outside the control, each 2 px: FocusStrokeColorOuter (`#E4000000` light, `#FFFFFF`
dark) outside FocusStrokeColorInner (`#B3FFFFFF` light, `#B3000000` dark), touching the control,
radius growing by the ring width. Measured on a ComboBox in the Gallery. Shown for keyboard
focus only. nd: `forms/toggle.rs` (`focus_visual`), `FormLook::{focus_ring, focus_inner}`.

## Scroll bars

ScrollBar_themeresources.xaml: a 2 px indicator 6 px into a 12 px bar while scrolling or while the
pointer moves over the view, fading (83 ms) 2 s after the last of either; under the pointer it
expands after 0.4 s over 167 ms on cubic-bezier(0,0,0,1) to a 6 px thumb on an
AcrylicInAppFillColorDefault track with 8 px caret buttons (EDDB, EDDC), and contracts 0.5 s after
the pointer leaves. Thumb ControlStrongFillColorDefault, min length 30, radius half its width.
Thumb drags, track pages, carets step 16. nd: `crates/nd-backend-gpui/src/scrollbar.rs`, on nd's
gesture scroll views (`render.rs`) and on GPUI-scrolled ones (`Build::scrolled`: forms, the
sidebar tree). Other presets draw none (`ScrollbarLook::of`).

## Motion

ControlFasterAnimationDuration 83 ms, ControlFastAnimationDuration 167 ms,
ControlNormalAnimationDuration 250 ms; enter curve cubic-bezier(0,0,0,1), exit (1,0,1,1).
Toggles move over 167 ms; flyouts and tooltips rise 8 px over 167 ms. Reduced motion follows the
system's client area animation setting. nd: `preset.rs` (`motion`), `FormLook::toggle_*`,
`overlay.rs`.

## Verifying

1. `ssh g815 'sudo systemctl start windows-vm'` (only with 30 GB or more free on g815); the guest
   is `ssh -J g815 -p 2222 kyan@127.0.0.1`, and GUI programs run as `ndvm` through a scheduled
   task (an ssh session has no desktop).
2. Build in the guest with debug info off (`CARGO_PROFILE_DEV_DEBUG=0`), the msvc toolchain and
   `--config "build.rustc-wrapper=''"`; check free space on g815 and the guest before and after.
3. Open the same control in the WinUI 3 Gallery and in nd's `gallery` example, both maximized at
   100% scale, and capture the VNC screen on g815 (`vncdo -s 127.0.0.1::5901 capture x.png`).
4. Crop both at native pixels, scale with point filtering side by side, and sample colours and
   edges with PIL. Walk rest, hover, pressed, focused and disabled, light and dark.
5. Linux goldens (`forms-fluent-*`, `layout-*-fluent-*`) hold the drawn look in CI:
   `ND_UPDATE_GOLDENS=1 cargo test -p nd-backend-gpui` through `scripts/linux-gate run`, then
   confirm only Fluent goldens moved.
