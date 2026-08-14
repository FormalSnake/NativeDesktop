---
title: App Icons
description: "One app.icon entry, converted and resized at package time: a macOS 26 layered Liquid Glass icon and the Linux hicolor theme set."
---

`app.icon` in `nativedesktop.config.ts` is the whole icon system. `nd package` reads whatever asset
you point it at and converts it to what each platform loads: `Assets.car` plus a legacy `.icns` on
macOS, the `hicolor` theme set plus the AppDir root icon on Linux. You never export sizes by hand.

```ts
export default defineConfig({
  app: {
    icon: { source: "assets/icon.png" },
  },
});
```

A bare string is shorthand for `source`. `macos` and `linux` override the shared source per
platform, and `layered` describes a macOS 26 icon built from separate art files.

## Per-platform overrides

`macos` and `linux` each win outright for their own platform, whatever else is set:

```ts
icon: {
  source: "assets/icon.svg",          // used wherever no override applies
  macos: "assets/MyApp.icns",         // wins on macOS
  linux: "assets/icon-linux.svg",     // wins on Linux
}
```

With no override for a platform, the two remaining sources are ranked by which is native there.
macOS prefers `layered` over `source`, because layered is its own format; Linux prefers `source`
over `layered`, because a layered composition has to be flattened to get there.

| Set | macOS uses | Linux uses |
| --- | --- | --- |
| `source` | `source` | `source` |
| `layered` | `layered` | `layered`, flattened |
| `source` + `layered` | `layered` | `source` |
| `macos` + `layered` | `macos` | `layered`, flattened |
| `linux` + `layered` | `layered` | `linux` |

The practical shape for most apps is `layered` on its own, with `linux` added only once the
flattened result is not what you want there.

## Sources and what each becomes

| Source | macOS | Linux |
| --- | --- | --- |
| `.png` | resized into a 10-size iconset, compiled to `<slug>.icns` | resized into 7 hicolor sizes + the AppDir root icon |
| `.svg` | rasterized to 1024px, then the PNG path | installed under `scalable/apps` + rasterized into the hicolor sizes |
| `.icon` | compiled with `actool` to `Assets.car` + `<name>.icns` | flattened to one SVG, then the SVG path |
| `layered` config | written to a `.icon`, then compiled | flattened to one SVG, then the SVG path |
| `.icns` | copied | not supported (set `linux` or `source` to a PNG/SVG) |
| `.iconset` | compiled with `iconutil` | not supported |

macOS sizes come out at 16, 32, 128, 256, 512 and 1024px (each with its @2x variant); Linux gets
16, 32, 48, 64, 128, 256 and 512px. Configuring no icon at all ships a 1x1 placeholder and prints
a one-time warning.

## macOS 26 layered icons

macOS 26 draws app icons from stacked layers rather than a flat bitmap, so the system can apply
Liquid Glass materials, specular highlights and shadows per layer and re-render the icon for
light, dark, tinted and clear appearances. The format is Apple's `.icon` bundle, which
[Icon Composer](https://developer.apple.com/icon-composer/) writes: a directory holding an
`icon.json` manifest next to an `Assets/` folder of layer art.

Point `macos` (or `source`) at a bundle you designed in Icon Composer:

```ts
icon: { macos: "assets/MyApp.icon", linux: "assets/icon.svg" }
```

Or describe the composition in config and let `nd package` build the bundle for you:

```ts
icon: {
  layered: {
    background: { gradient: ["#0a84ff", "#5e5ce6"] },
    layers: [
      "assets/shadow-glyph.svg",
      { image: "assets/glyph.svg", shadow: 0.4, translucency: 0.5, specular: true },
    ],
  },
}
```

- `background` is a solid color or a two-stop gradient running top to bottom. Colors are hex
  (`#0a84ff`, `#0a84ffcc`) or an Icon Composer color string (`display-p3:0,0.5,1,1`), which passes
  through untouched. Omit it for a transparent icon body.
- `layers` holds 1 to 4 entries, back to front, drawn full-bleed on the 1024x1024 grid Icon
  Composer uses. A bare string is shorthand for `{ image }`; `.svg` and `.png` are both accepted.
- Per layer, `specular` (default `true`), `translucency` (0..1, default `0.5`, `false` to disable)
  and `shadow` (0..1, default `0.5`, `false` to disable) control the glass treatment. Each layer
  becomes its own Icon Composer group, which is what makes those per-layer.

Either way the bundle is compiled with `xcrun actool`, which emits `Assets.car` (what macOS 26
renders) and a `.icns` (what earlier releases fall back to) into `Contents/Resources`, and
`Info.plist` gets both `CFBundleIconName` and `CFBundleIconFile`. The compile uses
`package.mac.minimumSystemVersion` as its deployment target.

The fallback `.icns` actool writes carries 16, 32, 128 and 256px only. If you ship to macOS 15 or
earlier and want a crisper Finder icon there, supply your own `macos: "assets/icon.icns"` instead.

Layered icons need Xcode installed, because `actool` ships with it. `nd doctor` reports this as an
`icon-tools` error before packaging fails.

## Linux icons

Linux has no equivalent of the layered format, so a `.icon` bundle or a `layered` config is
flattened into a single SVG: the background fill becomes the icon body and every layer composites on
top in order. The body follows the GNOME HIG grid, a 104x104 shape with a 24px corner radius on a
128x128 canvas, which on the 1024 grid is an 832px body inset by 96 with a 192px radius. That
mapping is what lets one composition read as native on both platforms with no per-platform art.

Two details of the flatten:

- SVG layers are inlined as nested `<svg>` elements rather than data URIs, because nested `<svg>`
  is plain SVG 1.1 that every rasterizer handles. Each layer's ids are namespaced first, so two
  layers that both define `id="gradient"` do not collide.
- An `automatic-gradient` fill in a hand-authored `icon.json` has no Linux equivalent, so it
  flattens to its seed color.

## Converters and degrade paths

Resizers are probed, never hard-required, so packaging works on a machine that only has some of
them:

| Job | Tools tried, in order |
| --- | --- |
| PNG resize (Linux) | `sips`, `magick`, `convert` |
| SVG rasterize | `rsvg-convert`, `magick`, `convert`, then `qlmanage` on macOS |
| PNG to `.icns` | `sips` + `iconutil` (macOS only) |
| `.icon` to `Assets.car` | `xcrun actool` (Xcode) |

`qlmanage` renders SVGs through QuickLook and ships with macOS, so an SVG source works on a stock
Mac with no extra installs. That is why `app.icon` can be a single SVG for both platforms.

On Linux, a missing resizer prints `ND_PACKAGE_ICON_SKIPPED reason=no-resizer` and installs the
source at its native size only; a resizer that fails mid-run drops the partial hicolor set, prints
`ND_PACKAGE_ICON_SKIPPED reason=resize-failed`, and keeps packaging. An SVG source does not print
either marker when no rasterizer is present: `scalable/apps` is a complete install on its own.

Run `nd doctor` to see which of these are available before you package.
