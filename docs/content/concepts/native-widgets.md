+++
title = "Native widgets"
description = "How an app defines its own widget with the NativeWidget trait: a drawn version for every OS and a native view where the OS has one."
+++

Write a widget as a plain function or `RenderOnce` struct when `div`, `text` and `canvas` are
enough. Implement `NativeWidget` when the OS has a real control you want, such as a map, or when
the widget must be a native view to behave like one. The framework lays out, focuses and
z-orders both kinds the same way, and a call site cannot tell your widget from a built-in. Every
built-in control in `nd-components` is written against this same trait.

The Places example has a map and a gauge. On macOS the map is MapKit's `MKMapView` through
objc2 and the gauge is a SwiftUI `Gauge` ([Swift](swift.md)). Elsewhere both are drawn.

<!-- docs:example native-widget -->

## The trait

A widget names its props, its events, what assistive technology and automation should see, and
its implementations:

<!-- docs:snippet examples/native-widget/src/map_view/mod.rs#map-widget -->

- `Props` is compared with `PartialEq` on every render; the native side is touched only when it
  changed.
- `Event` is what the widget can tell the app. A widget with no events uses
  `std::convert::Infallible`.
- `semantics` feeds the semantic tree, so tests and the MCP server see a "Map" with a place count
  and not an opaque view.
- `drawn` is the fallback for every backend that has no native version, and for the web. It is
  an ordinary element tree and can read `cx.theme()`.
- `backends` registers the native versions. `Backends::drawn_only()` is the default;
  `.appkit(..)` adds an AppKit one on macOS and `.swift(..)` a Swift one.

An element type wraps the props and implements `NativeElement` (`base_mut` and `into_parts`).
`NativeBase<W>` carries the id, layout style and event handlers, and `#[derive(IntoElement)]`
mounts it. Builder methods such as `.zoom(..)` and `.pins(..)` set props, and
`.on_region_change(..)` registers an event handler. See
[map_view/mod.rs](../../../examples/native-widget/src/map_view/mod.rs) for the full
set; the call site reads like any built-in:

<!-- docs:snippet examples/native-widget/src/main.rs#places-view -->

## The drawn fallback

The fallback in [drawn.rs](../../../examples/native-widget/src/map_view/drawn.rs) paints a 3x3
block of OpenStreetMap tiles around the centre with a `canvas`, draws the pins on top and pans on
drag. It is a real map, with the attribution OpenStreetMap requires, and it is the version the web
and Linux builds ship. Write it first: it defines the props and events, and the native version
then has to match them.

## The AppKit side

`AppKitWidget` creates an `NSView`, updates it when props change and reports its size:

<!-- docs:snippet examples/native-widget/src/map_view/appkit.rs#map-create -->

`AppKitCx` gives the main-thread marker, `emitter()` for delegates the view installs, and
`programmatic(..)`. `State` is whatever you want kept beside the view; here it is the delegate,
because `MKMapView` holds its delegate weakly. `update` receives the old and new props and applies
the difference:

<!-- docs:snippet examples/native-widget/src/map_view/appkit.rs#map-apply -->

`measure` returns the view's natural size. A map has none, so it returns zero and the app sizes it
with flex or explicit dimensions.

## Callbacks and controlled values

Two rules keep a native view from fighting the app's state.

No native callback calls app code. AppKit invokes delegates inside setters and layout:
`setRegion` fires `regionDidChange` before it returns, while the app's view is mid-update.
`Emit` queues the event, and the queue drains after the update, outside any borrow of the app's
state. Call `emit(..)` from a delegate and never run app code there.

<!-- docs:snippet examples/native-widget/src/map_view/appkit.rs#map-delegate -->

A controlled value never echoes. The map is controlled: the app owns `center` and `zoom`, and the
user's pan emits `RegionChanged`, which the app feeds back in. Without care that loops, and
MapKit makes it worse by fitting any region to its aspect ratio, so the region it reports is
never the one you set. `Synced<T>` holds what the native control holds. `apply` calls the setter
only when the new value differs from the last one applied or reported, `report` records what the
control produced, and the setter runs inside `cx.programmatic(..)` so the callback it triggers is
dropped. Feeding the reported region back then causes no second `setRegion`.

<!-- docs:snippet examples/native-widget/src/map_view/appkit.rs#map-viewport -->

Do the same for any prop the user can also change: a slider's step snapping and a split divider's
clamp are the same case.

## Build and run

The map crate depends on `objc2-map-kit` only for macOS, and the native-widget package is named
`places` with an `m6` feature that gates the binary:

```sh
cargo run -p places --features m6
```

`objc2` types are re-exported under `nd::native::appkit`, so no extra dependency is needed for
AppKit itself.
