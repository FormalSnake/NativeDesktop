---
title: Progress & Loading
description: ProgressCircle and Skeleton, a determinate ring and a shimmering placeholder, alongside the existing ProgressBar and Spinner.
---

Two widgets fill out the progress and loading vocabulary: `ProgressCircle` for a determinate ring
where a bar doesn't fit, and `Skeleton` for a shimmering placeholder while content loads. ProgressBar
and Spinner (linear-determinate and indeterminate) are already covered in the
[Widget Reference](/components/widget-reference/).

On macOS, ProgressBar, ProgressCircle, and Spinner are SwiftUI (`ProgressView`, `Gauge` in its
`.accessoryCircularCapacity` style, and `ProgressView` again), hosted the same way the rest of the
leaf widgets are. GTK still custom-draws ProgressCircle with Cairo, since Adwaita has no determinate
ring either.

## ProgressCircle (`<progresscircle>`)

A determinate ring. GTK custom-draws it (neither toolkit ships one natively there); macOS uses
SwiftUI's `Gauge` in its `.accessoryCircularCapacity` style, the system's own capacity ring.

![A ProgressCircle at a live fraction on macOS (AppKit)](../../../assets/screens/appkit/parity-progresscircle.png)

![A ProgressCircle at a live fraction on GNOME (GTK)](../../../assets/screens/gtk/parity-progresscircle.png)

```tsx
<progresscircle fraction={0.62} showLabel />;
```

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `fraction` | float | createAndUpdate | `0`–`1`. Default `0`. |
| `lineWidth` | int | createAndUpdate | Ring thickness in points/px. GTK only: `Gauge`'s ring stroke is system-drawn, so this prop is accepted but ignored on macOS. Default `3`. |
| `showLabel` | bool | createAndUpdate | Renders the percentage as centered text. Default `false`. |

Display-only, no events.

On macOS, `<spinner spinning={false}>` hides the indicator instead of leaving a static (non-spinning)
glyph on screen: SwiftUI's `ProgressView` has no public API to freeze an indeterminate spinner's
animation the way AppKit's `NSProgressIndicator.stopAnimation` did. GTK still shows a static glyph.

## Skeleton (`<skeleton>`)

A shimmering placeholder block for the shape of content that hasn't loaded yet, sized and rounded to
match what it stands in for.

![Skeleton placeholders standing in for an unloaded list on macOS (AppKit)](../../../assets/screens/appkit/parity-skeleton.png)

![Skeleton placeholders standing in for an unloaded list on GNOME (GTK)](../../../assets/screens/gtk/parity-skeleton.png)

```tsx
<skeleton width={140} height={14} />;
// A circular placeholder, e.g. for an avatar:
<skeleton width={40} height={40} radius={20} />;
```

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `width` | int | createAndUpdate | `0` sizes to the parent instead of a fixed width. Default `0`. |
| `height` | int | createAndUpdate | Default `16`. |
| `radius` | int | createAndUpdate | Corner radius. Default `6`; set to half of `width`/`height` for a circle. |
| `animated` | bool | createAndUpdate | Default `true`. The shimmer stays static under the OS reduce-motion setting regardless of this prop. |

Display-only, no events. Compose several into the shape of the row or card you're loading, and swap
them for the real content once it arrives. See `examples/parity/main.tsx`'s Loading section.
