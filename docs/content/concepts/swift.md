+++
title = "Swift"
description = "When and how an app adds Swift to its macOS build, using a SwiftUI gauge as the worked case."
+++

Most of AppKit needs no Swift: Rust calls it through objc2, and the [map widget](native-widgets.md)
is written that way. Add Swift only when the control you want exists in Swift or SwiftUI alone,
such as SwiftUI's `Gauge` or Swift Charts, or when it is a SwiftUI control Apple restyles with
each OS release, so hosting it keeps the app current. The rest of the app stays Rust, and off
macOS the widget's drawn version is used.

The Places example has a circular gauge written in SwiftUI and shown through `NSHostingView`.

<!-- docs:example native-widget -->

## Wire it up

Three pieces connect an app's Swift package to the Rust build.

1. List the package in `nativedesktop.toml`:

<!-- docs:snippet examples/native-widget/nativedesktop.toml#swift-packages -->

2. Call `nd::build::app()` from `build.rs`, with `nd` as a build dependency using the `build`
   feature. `nd new` writes this line into every template. On other targets it does nothing.

<!-- docs:snippet examples/native-widget/build.rs#build -->

3. Register the Swift view next to the drawn one with `Backends::swift`:

<!-- docs:snippet examples/native-widget/src/gauge/mod.rs#gauge-widget -->

`nd::build::app()` collects the framework's Swift packages and the app's own, generates one
aggregate SwiftPM package in `OUT_DIR`, builds one static library and links it into the binary.
Nothing is written into your source tree. The app's `Package.swift` declares a plain static
library with no dependencies; the script passes the framework's own package (`NDSwift`) to the
aggregate through `ND_SWIFT_PATH`.

## The Swift view

The Swift side exports C functions with `@_cdecl`, the same shape the framework's own Swift
uses: `create` returns a +1 retained view built from JSON props, `update` applies new props and
`measure` reports the fitting size. Props cross as JSON, which is fine because a widget's props
cross only when they change. `GaugeProps` has the same field names on both sides, and the Rust
props derive `serde::Serialize`, which `Backends::swift` requires.

The name given to `SwiftWidget::named` picks the symbols: `"Gauge"` is `nd_widget_gauge_create`,
`nd_widget_gauge_update` and `nd_widget_gauge_measure`, and a name of several words joins them
with underscores (`"MapView"` is `nd_widget_map_view_create`). The AppKit backend looks the
three up in the running binary the first time the widget mounts. When they are missing, because
the package is not in `swift_packages` or `build.rs` does not call `nd::build::app()`, it prints
the symbol it wanted and draws the widget instead.

<!-- docs:snippet examples/native-widget/swift/Widgets/Sources/Widgets/Gauge.swift#swift-widget -->

Pass universal state (enabled, tooltip, focus) into SwiftUI state yourself: a hosting view is not
the control the user sees.

Events do not cross from Swift yet: the C functions take no emitter, so a Swift widget's
`Event` is `Infallible`. A widget that reports user input writes its AppKit side in objc2, like
the map.

## Toolchain

Swift and the macOS SDK come from the system Xcode or Command Line Tools, never from Nix. The
minimum versions are pinned in [swift-toolchain.toml](../../../swift-toolchain.toml) (Swift 6, SDK
26). `nd::build::app()` fails with a message when the installed majors are lower.
The Swift runtime is the OS's own in `/usr/lib/swift`, present from macOS 14, so the example
declares `minimum_system_version = "14.0"`.

Inside the Nix dev shell the build resolves the SDK with `xcrun --sdk macosx --show-sdk-path` and
links with `/usr/bin/clang`, so no `SDKROOT` needs setting.

## When not to use Swift

- A control AppKit already has (`NSButton`, `NSTableView`, `MKMapView`): use objc2. It has no
  second toolchain and the call goes straight from Rust.
- A look you can compose from elements: write a composed widget, which runs on every OS.
- Anything that must also run on Linux or Windows: write the drawn version anyway. Swift only
  adds the macOS implementation.
