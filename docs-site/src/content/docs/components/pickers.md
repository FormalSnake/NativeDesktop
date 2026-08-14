---
title: Pickers
description: ColorPicker, DatePicker, and FontPicker, three widgets that open a native OS picker surface and report back a portable string value.
---

Three widgets wrap a native OS picker: a color well, a calendar, and the system font panel. Each
one's wire value is a plain portable string, so `NSColor`, `NSFont`, and their GTK equivalents
never cross NDP.

## ColorPicker (`<colorpicker>`)

`NSColorWell` in `.minimal` style (opening the shared `NSColorPanel`) on macOS, a GTK color button
on Linux. The value is a hex string, `#rrggbb` or `#rrggbbaa` when the color is not fully opaque,
the same convention `style.background` uses.

```tsx
const [color, setColor] = useState("#3366cc");

<colorpicker value={color} supportsAlpha onColorChanged={(e) => setColor(e.text)} />;
```

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `value` | string | createAndUpdate | Hex, default `#000000`. |
| `supportsAlpha` | bool | create | Shows the panel's alpha slider; without it the picker clamps to fully opaque. |

`colorChanged` → `onColorChanged` fires `{ text }` with the new hex string.

## DatePicker (`<datepicker>`)

Date only, no time component. The value is an ISO `YYYY-MM-DD` string, never a full timestamp.

```tsx
const [pickedDate, setPickedDate] = useState("");

<datepicker value={pickedDate} displayStyle="calendar" onDateChanged={(e) => setPickedDate(e.text)} />;
```

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `value` | string | createAndUpdate | ISO `YYYY-MM-DD`, or `""` for no selection. |
| `displayStyle` | `field` \| `calendar` | create | Default `calendar`. |
| `minDate` / `maxDate` | string | createAndUpdate | ISO bounds; out-of-range dates are unselectable. |

`dateChanged` → `onDateChanged` fires `{ text }` with the new ISO string.

`displayStyle="field"` (a compact text field with a dropdown calendar) is honored on macOS only,
where `NSDatePicker` in `.textFieldAndStepper` style is the closest fit. GTK always renders the
inline calendar. Use `calendar` for identical layout on both backends.

## FontPicker (`<fontpicker>`)

A button that opens the system font picker (`NSFontPanel` on macOS, `GtkFontDialogButton` on GTK)
and reports the chosen font back. The value is Pango font description syntax
(`"Family [Bold] [Italic] size"`, for example `"Sans Bold 14"`), which GTK already speaks natively.
The macOS side converts between Pango and `NSFont` so app code never has to.

```tsx
const [fontDesc, setFontDesc] = useState("Sans 12");

<fontpicker value={fontDesc} onFontChanged={(e) => setFontDesc(e.text)} />;
```

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `value` | string | createAndUpdate | Pango font description, default `"Sans 12"`. |

`fontChanged` → `onFontChanged` fires `{ text }` with the new Pango string.

Generic Pango family aliases resolve to a real font on both backends without naming an installed
typeface: `Sans`/`Sans-Serif`/`System` gives the system UI font, `Monospace`/`Mono` the system
monospace font, `Serif` the system serif design. An unrecognized family falls back to the system
font.

See `examples/gallery/main.tsx`'s Pickers tab for all three wired to live readouts, and the
[Widget Reference](/components/widget-reference/) for the generated prop tables.
