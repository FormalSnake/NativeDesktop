---
title: Form Controls
description: ToggleButton, SegmentedControl, NumberInput, LinkButton, and LevelIndicator — plus the boxed-list settings widgets SettingsGroup, Row, SwitchRow, and Clamp.
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
| `active` | bool | createAndUpdate | Controlled. Set it from `onToggled`. |

`toggled` → `onToggled` fires `{ checked }`.

## SegmentedControl (`<segmentedcontrol>`)

A fixed row of mutually-exclusive options (`AdwToggleGroup`-style on GTK, an `NSSegmentedControl`
on macOS). Unlike `<select>`, every option is visible at once. Reach for it when there are two to
five short labels, not a long list.

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

`activate` → `onActivate` fires `{ text: uri }` regardless of `openExternal`, so a link that should
navigate *inside* your app rather than in the OS browser leaves `openExternal` unset and handles the URI
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

No events. It is display-only.

See the [Widget Reference](/components/widget-reference/) for the generated prop tables and
`examples/gallery/main.tsx`'s "Controls" tab for all five wired to live state together.

## Boxed-list settings: SettingsGroup, Row, SwitchRow, Clamp

Preference pages are built from four structural widgets instead of hand-rolled `<box>` +
`<separator>` rows. On Linux they are the real libadwaita boxed-list stack (`AdwPreferencesGroup`,
`AdwActionRow`, `AdwSwitchRow`, `AdwClamp`); on macOS a SwiftUI grouped `Form` hosts the same rows
natively.

```tsx
<clamp maximumSize={560}>
  <settingsgroup title="General" description="Applied at next launch.">
    <switchrow title="Launch at login" checked={launch} onToggled={(e) => setLaunch(e.checked)} />
    <row title="Default folder" subtitle="Where new documents land">
      <select options={folders} selectedIndex={idx} onSelectionChanged={(e) => setIdx(e.index)} />
    </row>
    <row title="Check for updates" activatable onActivate={checkNow} />
  </settingsgroup>
</clamp>
```

- **`<settingsgroup>`** — `title` and `description` (both createAndUpdate) render as the group's
  native heading and footer. Row children join the rounded list; any other child lands below it.
  Reordering an already-mounted child settles in append order on Linux (`AdwPreferencesGroup` has
  no insert-at-index).
- **`<row>`** — `title`, `subtitle` (createAndUpdate), `iconName` (create), `activatable` (create;
  fires `onActivate` on click). Children mount into slots: `slot="prefix"` leads the row,
  the default `suffix` trails it. Suffix controls are vertically centered and keep their natural
  size; a `<slider>` should set `style={{ hexpand: true }}` for a usable track.
- **`<switchrow>`** — `title`/`subtitle` plus a controlled `checked` + `onToggled`, the boxed-list
  form of `<switch>`.
- **`<clamp>`** — single child, `maximumSize` (default 600) and `tighteningThreshold` (default 400,
  Linux only): the content column fills the window up to the ceiling, then stays centered, so forms
  don't stretch edge-to-edge when maximized.

`examples/settings/main.tsx` is the reference composition. The remaining Adwaita row family
(EntryRow, ComboRow, SpinRow, ExpanderRow) is planned; until then a `<row>` with a suffix
`<textinput>`/`<select>`/`<numberinput>` covers the same ground.
