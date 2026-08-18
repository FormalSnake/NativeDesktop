---
title: Form Controls
description: "ToggleButton, SegmentedControl, NumberInput, LinkButton, and LevelIndicator, plus the boxed-list settings widgets SettingsGroup, Row, SwitchRow, and Clamp."
---

Five input widgets beyond [Checkbox, Radio, Select, and
Slider](/components/widget-reference/), all cross-platform on AppKit and GTK/Adwaita, followed by
the four structural widgets a preferences page is built from.

On macOS, Checkbox, Switch, and Slider are SwiftUI (`Toggle` in its `.checkbox`/`.switch` style, and
`Slider`), hosted the same way the rest of the leaf widgets are. Radio stays plain AppKit
(`NSButton` in `.radio` mode): SwiftUI ships no composable radio-group control on macOS outside a
single `Picker`, and `NSButton`'s is already real system chrome, so there was nothing to gain from
replacing it. `<slider orientation="vertical">` on macOS lays the track out horizontally at a fixed
160pt length and rotates it, since SwiftUI has no vertical `Slider` axis; it does not fill the
available height the way the horizontal orientation fills available width.

## ToggleButton (`<togglebutton>`)

A button that stays pressed until clicked again (`GtkToggleButton`, or an `NSButton` in
`.pushOnPushOff` mode). Use it for a binary setting that should look like a button rather than a
switch.

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

A fixed row of mutually exclusive options (`AdwToggleGroup`-style on GTK, SwiftUI `Picker` in its
`.segmented` style on macOS). Every option is visible at once, unlike `<select>`. Good for two to
five short labels.

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

A bounded numeric stepper (`GtkSpinButton`, or `NSStepper` plus a text field on macOS) for integer
or fixed-decimal values with a known range. Saves validating a free-text `<textinput>` yourself.

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

A hyperlink-styled button (`GtkLinkButton`, or an `NSButton` with a link-style bezel). Clicking
always fires `onActivate` with the URI. Opening the link in the OS browser is a separate opt-in:

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

`activate` → `onActivate` fires `{ text: uri }` regardless of `openExternal`. For a link that should
navigate inside your app, leave `openExternal` unset and handle the URI in the handler.

## LevelIndicator (`<levelindicator>`)

A read-only bar meter (`GtkLevelBar` on GTK, a custom `NSView` on macOS) for a bounded value with
optional warning and critical thresholds: disk usage, battery, signal strength. No user
interaction; drive it from whatever produces the value.

```tsx
<levelindicator min={0} max={1} value={0.72} warningValue={0.6} criticalValue={0.85} />;
```

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `value` | float | createAndUpdate | |
| `min` / `max` | float | create | Defaults `0`/`1`. |
| `warningValue` / `criticalValue` | float | createAndUpdate | Value thresholds past which the fill recolors (amber/red). Omit either to skip that tier. |
| `discrete` | bool | create | Render as fixed segments (like a battery icon) instead of a continuous fill. |

Display-only, no events.

See the [Widget Reference](/components/widget-reference/) for the generated prop tables and
`examples/gallery/main.tsx`'s Controls tab for all five wired to live state.

## Boxed-list settings: SettingsGroup, Row, SwitchRow, Clamp

Build preference pages from these four widgets rather than hand-rolled `<box>` and `<separator>`
rows. On Linux they are the real libadwaita boxed-list stack (`AdwPreferencesGroup`,
`AdwActionRow`, `AdwSwitchRow`, `AdwClamp`). On macOS a SwiftUI grouped `Form` hosts the same rows
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

- **`<settingsgroup>`**: `title` and `description` (both createAndUpdate) render as the group's
  native heading and footer. Row children join the rounded list; any other child lands below it.
  Reordering an already-mounted child settles in append order on Linux, since
  `AdwPreferencesGroup` has no insert-at-index.
- **`<row>`**: `title`, `subtitle`, `iconData` (createAndUpdate), `iconName` (create),
  `activatable` (create, fires `onActivate` on click). `iconData` is raw image bytes — a
  `data:<mime>;base64,<payload>` URL or a bare base64 payload, the shape `faviconChanged` hands
  you — for an icon no theme has, and it wins over `iconName` when both are set; a payload the
  platform cannot decode draws no icon and logs one `ND_WARN`. Children mount into slots:
  `slot="prefix"` leads the row, the default `suffix` trails it. Suffix controls are vertically
  centered and keep their natural size; give a `<slider>` `style={{ hexpand: true }}` for a usable
  track.
- **`<switchrow>`**: `title` and `subtitle` plus a controlled `checked` and `onToggled`. The
  boxed-list form of `<switch>`.
- **`<clamp>`**: single child, `maximumSize` (default 600) and `tighteningThreshold` (default 400,
  Linux only). The content column fills the window up to the ceiling, then stays centered, so forms
  do not stretch edge to edge when maximized.

`examples/settings/main.tsx` is the reference composition. The rest of the Adwaita row family
(EntryRow, ComboRow, SpinRow, ExpanderRow) is planned. Until then, a `<row>` with a suffix
`<textinput>`, `<select>`, or `<numberinput>` covers the same ground.
