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

Declare `@nativedesktop/native` as a dependency and create `nativedesktop.config.ts`:

```ts
import { defineConfig } from "nd/config";

export default defineConfig({ native: { plugins: [{
  darwin: "native/build/libapp.dylib",
  linux: "native/build/libapp.so",
  build: {
    darwin: {
      command: ["bash", "-c", "mkdir -p native/build && /usr/bin/xcrun --sdk macosx swiftc -emit-library -o native/build/libapp.dylib -I \"$ND_NATIVE_PACKAGE/macos\" native/macos/App.swift \"$ND_NATIVE_PACKAGE/macos/NativeDesktopNative.swift\" -framework AppKit -framework SwiftUI"],
      inputs: ["native/macos", "node_modules/@nativedesktop/native"],
    },
    linux: {
      command: ["bash", "-c", "mkdir -p native/build && cc -shared -fPIC $(pkg-config --cflags gtk4) -I \"$ND_NATIVE_PACKAGE/include\" -I \"$ND_NATIVE_PACKAGE/linux\" native/linux/app.c -o native/build/libapp.so $(pkg-config --libs gtk4)"],
      inputs: ["native/linux", "node_modules/@nativedesktop/native"],
    },
  },
}] } });
```

`nd dev` and `nd build` run only these app-owned commands when inputs are newer than the output. They never invoke `zig build` or rebuild the host. The resulting paths are passed through `ND_PLUGIN_PATHS`; legacy `ND_PLUGIN_PATH` remains supported.

Build commands run with `ND_NATIVE_PACKAGE` set to the installed `@nativedesktop/native` package root; on macOS the child environment also drops `SDKROOT` and Nix compiler variables and sets `DEVELOPER_DIR` from `xcode-select -p`, so plain `xcrun swiftc` finds the real Xcode toolchain. `inputs` are literal paths resolved from the app directory — no env expansion — so reference the package through `node_modules/@nativedesktop/native`.

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

Return an AppKit `NSView`. `-I "$ND_NATIVE_PACKAGE/macos"` provides the `CNdPlugin` Clang module (the C ABI header), and compiling `$ND_NATIVE_PACKAGE/macos/NativeDesktopNative.swift` into the dylib provides the Swift helpers: conform to `NativeDesktopView`, return `NativeDesktopPlugin.descriptor(...)` from `nd_plugin_entry`, and call `NativeDesktopPlugin.registerView` from its initialize callback (one view kind per plugin; hand-roll an `nd_view_impl` to register more). For SwiftUI, `NativeDesktopSwiftUIView` hosts a SwiftUI tree behind that protocol. This follows the same ownership rule as the framework's SettingsGroup bridge: NativeDesktop retains one AppKit identity while SwiftUI controls content/placement inside it.

Production `.app` packaging must embed the app dylib and include it in signing/notarization. The development loader accepts an absolute build path.

## Linux

Build a `.so` against GTK 4 and the stable C header. GTK development headers and `pkg-config gtk4` are required on the app developer's machine. `nd_native_gtk.h` (via `-I "$ND_NATIVE_PACKAGE/linux"`) carries `nd_gtk_view_state`, `nd_gtk_connect_state`, and `nd_gtk_emit` for per-view registry/node bookkeeping. Allocate/free component state in the app library; do not rely on framework allocators.

See `examples/nativeview-demo` for app-owned GTK and AppKit implementations, typed React props, an event, prop updates, configuration, and cached build commands.
