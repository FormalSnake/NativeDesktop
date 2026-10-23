+++
title = "Native widgets"
description = "Define your own widget with the NativeWidget trait: a drawn version for every OS, and a native view where the OS has one."
+++

When `div`, `text`, and `canvas` are enough, write a widget as a plain function or a `RenderOnce`
struct. Implement `NativeWidget` when the OS has a real control that you want, such as a map, or
when the widget must be a native view to behave like one. The framework handles layout, focus, and
z-order the same way for both kinds, and a call site can't tell your widget from a built-in one.
Every built-in control in `nd-components` uses this same trait.

The Places example has a map and a gauge. On macOS, the map is MapKit's `MKMapView` through
objc2, and the gauge is a SwiftUI `Gauge` ([Swift](swift.md)). On other platforms, nd draws both.

On AppKit, a widget with an objc2 or Swift implementation mounts as its own view. The backend
frames, measures, and updates that view like a built-in control. Everywhere else, and on AppKit
when a Swift widget's symbols aren't in the binary, the element mounts its drawn tree inside a
container that takes the element's id, layout style, and semantics.

<!-- docs:example native-widget -->

## The trait

A widget names its props, its events, what assistive technology and automation see, and its
implementations:

<!-- docs:snippet examples/native-widget/src/map_view/mod.rs#map-widget -->

- nd compares `Props` with `PartialEq` on every render, and touches the native side only when
  the props changed.
- `Event` is what the widget can tell the app. A widget with no events uses
  `std::convert::Infallible`.
- `semantics` feeds the semantic tree, so tests and the MCP server see a "Map" with a place count
  instead of an opaque view.
- `drawn` is the fallback for the web and for every backend that has no native version. It's an
  ordinary element tree, and it can read `cx.theme()`.
- `backends` registers the native versions. `Backends::drawn_only()` is the default.
  `.appkit(..)` adds an AppKit version on macOS, and `.swift(..)` adds a Swift version.

An element type wraps the props and implements `NativeElement` (`base_mut` and `into_parts`).
`NativeBase<W>` carries the id, layout style, and event handlers, and `#[derive(IntoElement)]`
mounts it. Builder methods such as `.zoom(..)` and `.pins(..)` set props, and
`.on_region_change(..)` registers an event handler. For the full set, see
[map_view/mod.rs](../../../examples/native-widget/src/map_view/mod.rs). The call site reads like
any built-in:

<!-- docs:snippet examples/native-widget/src/main.rs#places-view -->

## The drawn fallback

The fallback in [drawn.rs](../../../examples/native-widget/src/map_view/drawn.rs) uses a `canvas`
to paint the OpenStreetMap tiles that cover the view, and a second `canvas` to draw the pins on top.
It fits the region to its size the way MapKit does, so both show the same places, and it pans when
you drag. It's a real map, with the attribution that OpenStreetMap requires, and the web, Linux and
Windows builds ship it. The image pipeline downloads each tile once into its disk cache. Write the fallback first. It defines the props and events, and the native
version then has to match them.

## The AppKit side

`AppKitWidget` creates an `NSView`, updates it when the props change, and reports its size:

<!-- docs:snippet examples/native-widget/src/map_view/appkit.rs#map-create -->

`AppKitCx` gives you the main-thread marker, `emitter()` for delegates that the view installs, and
`programmatic(..)`. `State` is whatever you want to keep beside the view. Here, it's the delegate,
because `MKMapView` holds its delegate weakly. `update` receives the old and new props and applies
the difference:

<!-- docs:snippet examples/native-widget/src/map_view/appkit.rs#map-apply -->

`measure` returns the view's natural size. A map has no natural size, so it returns zero, and the
app sizes it with flex or explicit dimensions.

## Callbacks and controlled values

Two rules keep a native view from fighting the app's state.

No native callback calls app code. AppKit invokes delegates inside setters and layout:
`setRegion` fires `regionDidChange` before it returns, while the app's view is in the middle of an
update. `Emit` queues the event, and the queue drains after the update, outside any borrow of the
app's state. Call `emit(..)` from a delegate, and never run app code there.

<!-- docs:snippet examples/native-widget/src/map_view/appkit.rs#map-delegate -->

A controlled value never echoes. The map is controlled: the app owns `center` and `zoom`, and
when the user pans, the map emits `RegionChanged`, which the app feeds back in. Without care, that
loops. MapKit makes it worse, because it fits any region to its aspect ratio, so the region that it
reports is never the one you set. `Synced<T>` holds what the native control holds:

- `apply` calls the setter only when the new value differs from the last one that was applied or
  reported.
- `report` records what the control produced.
- The setter runs inside `cx.programmatic(..)`, so nd drops the callback that it triggers.

When the app feeds the reported region back, no second `setRegion` happens.

<!-- docs:snippet examples/native-widget/src/map_view/appkit.rs#map-viewport -->

Do the same for any prop that the user can also change. A slider's step snapping and a split
divider's clamp are the same case.

## Build and run

The map crate depends on `objc2-map-kit` only for macOS. The native-widget package is named
`places`, and an `m6` feature gates the binary:

```sh
cargo run -p places --features m6
```

nd re-exports the `objc2` types under `nd::native::appkit`, so you don't need an extra dependency
for AppKit itself.
