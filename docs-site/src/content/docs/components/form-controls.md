---
title: Form Controls
description: ToggleButton, SegmentedControl, NumberInput, LinkButton, and LevelIndicator — five widgets for input shapes Checkbox/Radio/Slider don't cover.
---

Five widgets round out the input vocabulary beyond [Checkbox, Radio, Select, and
Slider](/components/widget-reference/): a persistent-pressed button, a segmented single-choice
control, a bounded numeric stepper, a hyperlink-styled button, and a read-only bar meter. All five
are cross-platform (AppKit + GTK/Adwaita).

## ToggleButton (`<togglebutton>`)

A button that stays pressed until clicked again (`GtkToggleButton` / an `NSButton` in
`.pushOnPushOff` mode) for a binary setting you want to look like a button, not a switch.

```tsx
const [bold, setBold] = useState(false);

<togglebutton label="Bold" active={bold} onToggled={(e) => setBold(e.checked)} />;
```

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `label` | string | createAndUpdate | |
| `iconName` | string | create | |
| `active` | bool | createAndUpdate | Controlled — set it from `onToggled`. |

`toggled` → `onToggled` fires `{ checked }`.

## SegmentedControl (`<segmentedcontrol>`)

A fixed row of mutually-exclusive options (`AdwToggleGroup`-style on GTK, an `NSSegmentedControl`
on macOS). Unlike `<select>`, every option is visible at once; reach for it when there are 2–5 short
labels, not a long list.

```tsx
const [sizeIndex, setSizeIndex] = useState(1);

<segmentedcontrol
  options={["Small", "Medium", "Large"]}
  selectedIndex={sizeIndex}
  onSelectionChanged={(e) => setSizeIndex(e.index)}
/>;
```

`options` (stringList) is create-only, same as `<select>`. `selectionChanged` → `onSelectionChanged`
fires `{ index }`.

## NumberInput (`<numberinput>`)

A bounded numeric stepper (`GtkSpinButton` / `NSStepper`+text on macOS) for integer or fixed-decimal
values with a known range, instead of a free-text `<textinput>` you'd have to validate yourself.

```tsx
const [seats, setSeats] = useState(4);

<numberinput value={seats} min={1} max={10} step={1} digits={0}
  onValueChanged={(e) => setSeats(e.value)} />;
```

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `value` | float | createAndUpdate | |
| `min` / `max` / `step` | float | create | |
| `digits` | int | create | Decimal places shown (`0` for integers). |
| `wraps` | bool | create | Wrap from `max` back to `min` (and vice versa). |

`valueChanged` → `onValueChanged` fires `{ value }`.

## LinkButton (`<linkbutton>`)

Renders as a hyperlink-styled button (`GtkLinkButton` / `NSButton` with a link-style bezel). It
always fires `onActivate` with the URI when clicked; opening the link in the OS browser is a
separate opt-in, not a side effect of clicking:

```tsx
const [lastActivated, setLastActivated] = useState("");

<linkbutton
  label="NativeDesktop Docs"
  uri="https://nativedesktop.dev"
  openExternal // omit this to only observe onActivate, e.g. for in-app navigation
  onActivate={(e) => setLastActivated(e.text)}
/>;
```

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `label` | string | createAndUpdate | |
| `uri` | string | createAndUpdate | |
| `visited` | bool | createAndUpdate | Purely visual (dims the link). |
| `openExternal` | bool | createAndUpdate | Default `false`. When `true`, activating also opens `uri` in the OS default browser (`open`/`xdg-open`, the same mechanism as [`openExternal()`](/native-platform/system-capabilities/#shell-helpers)). |

`activate` → `onActivate` fires `{ text: uri }` regardless of `openExternal` — so a link that should
navigate *inside* your app (not the OS browser) just leaves `openExternal` unset and handles the URI
in the handler.

## LevelIndicator (`<levelindicator>`)

A read-only bar meter (`GtkLevelBar` on GTK, a custom `NSView` on macOS) for showing a bounded value
with optional warning/critical thresholds (disk usage, battery, signal strength). There's no user
interaction; drive it from whatever produces the value, often a `<slider>` in a demo:

```tsx
<levelindicator min={0} max={1} value={0.72} warningValue={0.6} criticalValue={0.85} />;
```

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `value` | float | createAndUpdate | |
| `min` / `max` | float | create | Defaults `0`/`1`. |
| `warningValue` / `criticalValue` | float | createAndUpdate | Value thresholds past which the fill recolors (amber/red). Omit either to skip that tier. |
| `discrete` | bool | create | Render as fixed segments (like a battery icon) instead of a continuous fill. |

No events — it's display-only.

See the [Widget Reference](/components/widget-reference/) for the generated prop tables and
`examples/gallery/main.tsx`'s "Controls" tab for all five wired to live state together.
