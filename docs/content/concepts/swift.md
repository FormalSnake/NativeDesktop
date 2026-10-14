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
`NDKit`, the Swift side of the framework, is compiled once and nothing is written into your source
tree. The package is built only through that script: its `Package.swift` finds `NDKit` through
the `ND_KIT_PATH` environment variable the script sets.

## The Swift view

A widget conforms to `NDSwiftWidget` and is exported with `#ndExport`. `make` creates the view,
`update` applies new props, `measure` reports its size and `value` is the semantic value
automation reads. `Props` and `Event` are `Codable`; `Never` pairs with Rust's `Infallible`.

<!-- docs:snippet examples/native-widget/swift/Widgets/Sources/Widgets/Gauge.swift#swift-widget -->

The props struct in Swift mirrors the Rust props (`GaugeProps` on both sides, same field names).
Props and events cross as JSON, which is fine because a widget's props cross only when they
change. After that, layout, focus, input, overlays, semantics and the override modifiers work as
they do for an objc2 widget. Pass universal state (enabled, tooltip, focus) into SwiftUI state
yourself: a hosting view is not the control the user sees.

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
