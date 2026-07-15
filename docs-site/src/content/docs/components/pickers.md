---
title: Pickers
description: ColorPicker, DatePicker, and FontPicker — three widgets that open a native OS picker surface and report back a portable string value.
---

Three widgets each wrap a native OS picker — a color well, a calendar, the system font panel — and
converge on one design: the wire value is always a plain, portable **string**, so the picker's own
native representation (`NSColor`, `NSFont`, ...) never crosses the NDP protocol.

## ColorPicker (`<colorpicker>`)

Wraps `NSColorWell` (`.minimal` style, opens the shared `NSColorPanel`) on macOS and a GTK color
button on Linux. The wire value is a hex string — `#rrggbb`, or `#rrggbbaa` when the color isn't
fully opaque — the same convention `style.background` and friends already use.

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

Date-only — there's no time component. The wire value is a **date-only ISO string**
(`YYYY-MM-DD`), never a full timestamp.

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

`displayStyle="field"` (a compact text field with a dropdown calendar) is honored on **macOS only**
(`NSDatePicker` in `.textFieldAndStepper` style is the closest fit); **GTK always renders the inline
calendar** regardless of `displayStyle` — there's no compact-field GTK counterpart wired up yet. Pick
`calendar` if you want identical layout on both backends.

## FontPicker (`<fontpicker>`)

Wraps a button that opens the OS's system font picker (`NSFontPanel` on macOS, `GtkFontDialogButton`
on GTK) and reports the chosen font back. Rather than invent a third font-description format, the
wire value is **canonical Pango font description syntax** — `"Family [Bold] [Italic] size"`, e.g.
`"Sans Bold 14"` — the same string GTK already speaks natively; the macOS side does the Pango↔`NSFont`
conversion so the app code never has to.

```tsx
const [fontDesc, setFontDesc] = useState("Sans 12");

<fontpicker value={fontDesc} onFontChanged={(e) => setFontDesc(e.text)} />;
```

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `value` | string | createAndUpdate | Pango font description, default `"Sans 12"`. |

`fontChanged` → `onFontChanged` fires `{ text }` with the new Pango string.

Generic Pango family aliases resolve to a real font on both backends without you naming a specific
installed typeface: `Sans`/`Sans-Serif`/`System` → the system UI font, `Monospace`/`Mono` → the system
monospace font, `Serif` → the system serif design. An unrecognized family name falls back to the
system font rather than failing.

See `examples/gallery/main.tsx`'s "Pickers" tab for all three wired to live readouts, and the
[Widget Reference](/components/widget-reference/) for the generated prop tables.
