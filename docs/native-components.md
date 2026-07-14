# App-owned native components

NativeDesktop apps can ship native UI without rebuilding NativeDesktop. The prebuilt host loads app-owned shared libraries at launch; the generic `NativeView` widget routes lifecycle operations to a factory registered by that library.

## React API

```tsx
import { defineNativeComponent, type NativeComponentRef } from "@nativedesktop/react";

const MapView = defineNativeComponent<
  { latitude: number; longitude: number },
  { latitude: number; longitude: number },
  { animated?: boolean }
>({ viewKind: "com.example.map" });

<MapView
  props={{ latitude: 51.5, longitude: -0.1 }}
  onNativeEvent={({ name, data }) => console.log(name, data)}
/>
```

`props` is JSON-serialized. A ref exposes `send(command, arg)`, or call `sendNativeCommand(ref.current, command, arg)`. Native events arrive as `{ name, data }`. NativeView is an opaque leaf: React children cannot be mounted inside it.

## Configuration and builds

Create `nativedesktop.config.ts`:

```ts
import { defineConfig } from "nd/config";

export default defineConfig({ native: { plugins: [{
  darwin: "native/build/libapp.dylib",
  linux: "native/build/libapp.so",
  build: {
    darwin: { command: ["swift", "build", "-c", "release"], inputs: ["native/macos"] },
    linux: { command: ["bash", "-c", "cc -shared …"], inputs: ["native/linux"] },
  },
}] } });
```

`nd dev` and `nd build` run only these app-owned commands when inputs are newer than the output. They never invoke `zig build` or rebuild the host. The resulting paths are passed through `ND_PLUGIN_PATHS`; legacy `ND_PLUGIN_PATH` remains supported.

## ABI

Use `@nativedesktop/native/include/nd_plugin.h`. Export `nd_plugin_entry()` and declare ABI v3. Register an `nd_view_impl` in `init()`:

- `create(props_json)` returns `GtkWidget*` on Linux or `NSView*` on macOS.
- `apply_props` handles later React props.
- `connect(view, node_id)` records the node identity.
- `command` handles imperative commands.
- `destroy` releases app-owned state exactly once.
- `registry->emit_event(registry, node_id, name, payload_json)` emits an event.

The registry pointer and callbacks remain valid for the loaded plugin lifetime. Calls that touch widgets occur on the platform UI thread. ABI v1 command plugins and ABI v2 view plugins continue to load.

## macOS

Return an AppKit `NSView`. For SwiftUI, wrap your view with `NSHostingView`; `packages/native/macos/NativeDesktopNative.swift` includes a small `NativeDesktopSwiftUIView` helper. This follows the same ownership rule as the framework's SettingsGroup bridge: NativeDesktop retains one AppKit identity while SwiftUI controls content/placement inside it.

Production `.app` packaging must embed the app dylib and include it in signing/notarization. The development loader accepts an absolute build path.

## Linux

Build a `.so` against GTK 4 and the stable C header. GTK development headers and `pkg-config gtk4` are required on the app developer's machine. Allocate/free component state in the app library; do not rely on framework allocators.

See `examples/nativeview-demo` for app-owned GTK and AppKit implementations, typed React props, an event, prop updates, configuration, and cached build commands.
