# Windows look: gaps against WinUI 3

Each gap found by comparing nd's `gallery` example with the WinUI 3 Gallery (3.0.4.0) in the g815
Windows VM at 100% scale, light theme, unless noted. Ticked items say how the fix was verified.
The rules and values are in [docs/dev/hig/windows.md](../dev/hig/windows.md).

## Controls

- [x] Text fields had a 2 px uniform border. Now 1 px ControlStroke with a ControlStrongStroke
  bottom edge, a 2 px accent underline when focused, InputActive fill. VM capture: fill, top
  `#e5e5e5` and bottom `#868686` edges match the Gallery's TextBox.
- [x] Check box: 2 px grey border, no hover or pressed fills. Now 1 px strong stroke,
  ControlAltFill states, 12 px glyph. VM capture beside the CheckBox page.
- [x] Toggle switch: off fill and outline colours, no knob growth. Now ControlAltFillSecondary
  off fill, strong outline, knob 12/14/17x14. Pixel-sampled against the ToggleSwitch page.
- [x] Radio button: 6 px dot, no states. Now 12/14/10 dot, 32 px rows 8 apart. VM capture
  side by side with the RadioButton page (40 px pitch, same ring and dot).
- [x] Slider: plain white knob. Now a 20 px thumb with a 1 px elevation border and a 12/14/10
  accent dot. VM capture.
- [x] Progress bar 4 px track; ring grey with a track. Now 1 px rail with a 3 px bar; ring
  accent, no track, 32 px. VM capture.
- [x] Combo box: no stroke, 16 px black chevron, menu below with a trailing check. Now elevation
  border, 12 px secondary chevron, menu over the button with a selection pill and subtle fills,
  as wide as the button. VM capture beside the Gallery's open ComboBox. Editable combo box too.
- [x] Buttons: fills, elevation edge and 32 px height. VM capture beside the Button page.
- [x] Focus: a single accent ring. Now WinUI's two 2 px rings. Pixel-sampled on a focused
  ComboBox.
- [x] No scroll bars on the GPUI backend. Now WinUI's ScrollBar (indicator, expansion, carets,
  drag, paging) on gesture and GPUI-scrolled views. VM capture after a wheel scroll and on hover.
- [x] Stepper drew minus and plus outside the field. Now a NumberBox with inline chevron
  buttons. VM capture beside the ProgressBar page's NumberBox.
- [x] Links underlined on hover. Now HyperlinkButton: padding, subtle fill, no underline.
- [x] Pointing hand over every control. Windows keeps the arrow; links keep the hand.
- [x] Banner was an accent-container strip. Now the informational InfoBar (attention
  background, card stroke, accent badge, standard button). VM capture beside the InfoBar page.
- [x] Disabled opacity 0.5. Now 0.4.
- [x] TextBox header (form field label) was semibold. Now Body (14 regular) with 8 px to the
  control under the Fluent look (`form_field`, nd-components). Built; not yet captured.
- [ ] Segmented control: WinUI has none built in; nd's selected segment lacks the toolkit
  Segmented's accent indicator. Decide whether to match the toolkit or SelectorBar.
- [ ] Time field draws a small stepper; WinUI's TimePicker opens a looping flyout.
- [x] Text selection was a pale accent tint under unchanged text. Now solid accent under white
  text (the accent container in dark, where the accent is too light for white), the selected
  run recoloured (`FormLook::selection_text`, `recolor` with a unit test). Built; not yet
  captured.

## Navigation and chrome

- [x] NavigationView rows were 33 px apart with a leading disclosure triangle. Now 36 px rows
  4 apart, icon 12 in, trailing 12 px chevron, Body Strong headers, pill at the item's
  indentation. VM capture side by side with the Gallery's pane.
- [x] Context menus were 4 px padded rows on the control radius with a generic border and
  shadow. Now MenuFlyout metrics (32 px items inset 4,2, padding 11, subtle hover and pressed
  fills, 28 px check column, overlay radius) and the flyout stroke and shadow, which popovers
  and tooltips share. Values from MenuFlyout_themeresources.xaml; not yet captured beside the
  Gallery (the VM powered off on low disk).
- [x] Command palette highlighted its row with the accent container. Now WinUI's list
  selection (subtle fill, accent pill, text unchanged). Built; not yet captured.
- [ ] ListView: hover and selection pill against the ListView page.
- [ ] ToolTip, ContentDialog, TabView, Expander: compare against their pages.
- [ ] Pending VM captures (the VM needs 45 GB free on g815 to start): dark theme for every
  control above; context menu, popover, palette, field label and selection beside the Gallery;
  the `real_windows_job` run without the job object, which should fail.
- [ ] Settings, Task Manager and Notepad captures for title bar, layer and sidebar.
- [ ] Windows has no window menu for windows the chrome is not installed on (TODO.md).
  Analysis: every window gets `Chrome` in `WindowsHooks::window_opened`; only a failed install
  skips it, and that window keeps GPUI's `WS_SYSMENU`, so an Alt+Space GPUI leaves unhandled
  reaches `DefWindowProc` and the system menu. GPUI handles `WM_NCRBUTTONUP` itself, so the
  title-row right-click may still show nothing there. Needs a VM run that forces the install to
  fail.

## Interaction

- [ ] Access keys (Alt shows key tips). Deferred: nd has no access-key property on elements yet.
  It needs one in nd-ui (set per control, as WinUI's `AccessKey`), carried to the GPUI backend,
  plus a key-tip overlay shown on a lone Alt press and dispatching the matching control's press
  action. Not something the skin can add on its own.
- [ ] Flyout and page transitions against WinUI's.

## Tooling

- [x] A Windows test run left `hello.exe` processes alive. `nd-test`'s launch puts each app in a
  job object that kills it, and the crash handler it starts, when the test process exits.
  `real_windows_job` (nd-test) leaks an `AppHandle` in a child test process and checks hello
  ends with it; passes in the VM.
