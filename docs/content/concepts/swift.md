+++
title = "Swift"
description = "Add Swift to your app's macOS build when you need it, with a SwiftUI gauge as the worked example."
+++

Most of AppKit needs no Swift. Rust calls it through objc2, and the [map widget](native-widgets.md)
works that way. Add Swift only in these cases:

- The control you want exists only in Swift or SwiftUI, such as SwiftUI's `Gauge` or Swift Charts.
- It's a SwiftUI control that Apple restyles with each OS release, so hosting it keeps your app
  current.

The rest of the app stays in Rust, and on other platforms, nd uses the widget's drawn version.

The Places example has a circular gauge written in SwiftUI and shown through `NSHostingView`.

<!-- docs:example native-widget -->

## Wire it up

To connect your app's Swift package to the Rust build:

1. List the package in `nativedesktop.toml`:

<!-- docs:snippet examples/native-widget/nativedesktop.toml#swift-packages -->

2. Call `nd::build::app()` from `build.rs`, with `nd` as a build dependency that uses the `build`
   feature. `nd new` writes this line into every template. On other targets, it does nothing.

<!-- docs:snippet examples/native-widget/build.rs#build -->

3. Register the Swift view next to the drawn one with `Backends::swift`:

<!-- docs:snippet examples/native-widget/src/gauge/mod.rs#gauge-widget -->

`nd::build::app()` collects the framework's Swift packages and your app's own, and generates one
aggregate SwiftPM package in `OUT_DIR`. It builds one static library and links it into the binary.
It writes nothing into your source tree. Your app's `Package.swift` declares a plain static
library with no dependencies. The build script passes the framework's own package (`NDSwift`) to
the aggregate through `ND_SWIFT_PATH`.

## The Swift view

The Swift side exports C functions with `@_cdecl`, in the same shape that the framework's own
Swift uses:

- `create` returns a +1 retained view built from JSON props.
- `update` applies new props.
- `measure` reports the fitting size.

Props cross as JSON. That's fine, because a widget's props cross only when they change.
`GaugeProps` has the same field names on both sides, and the Rust props derive `serde::Serialize`,
which `Backends::swift` requires.

The name that you give `SwiftWidget::named` picks the symbols. `"Gauge"` becomes
`nd_widget_gauge_create`, `nd_widget_gauge_update`, and `nd_widget_gauge_measure`. A name of
several words joins them with underscores: `"MapView"` becomes `nd_widget_map_view_create`. The
AppKit backend looks up the three symbols in the running binary the first time the widget mounts.
If they're missing, because the package isn't in `swift_packages` or `build.rs` doesn't call
`nd::build::app()`, the backend prints the symbol it wanted and draws the widget instead.

<!-- docs:snippet examples/native-widget/swift/Widgets/Sources/Widgets/Gauge.swift#swift-widget -->

Pass universal state (enabled, tooltip, focus) into SwiftUI state yourself, because a hosting view
isn't the control that the user sees.

Events don't cross from Swift yet. The C functions take no emitter, so a Swift widget's `Event` is
`Infallible`. If your widget reports user input, write its AppKit side in objc2, like the map.

## Toolchain

Swift and the macOS SDK come from the system Xcode or Command Line Tools, never from Nix.
[crates/nd/swift-toolchain.toml](../../../crates/nd/swift-toolchain.toml) pins the minimum versions (Swift 6, SDK 26).
When the installed major versions are lower, `nd::build::app()` fails with a message. The Swift
runtime is the OS's own in `/usr/lib/swift`, which is present from macOS 14 on, so the example
declares `minimum_system_version = "14.0"`.

Inside the Nix dev shell, the build resolves the SDK with `xcrun --sdk macosx --show-sdk-path` and
links with `/usr/bin/clang`, so you don't need to set `SDKROOT`.

## When not to use Swift

- For a control that AppKit already has (`NSButton`, `NSTableView`, `MKMapView`), use objc2. It
  needs no second toolchain, and the call goes straight from Rust.
- For a look that you can compose from elements, write a composed widget, which runs on every OS.
- For anything that must also run on Linux or Windows, write the drawn version anyway. Swift only
  adds the macOS implementation.
