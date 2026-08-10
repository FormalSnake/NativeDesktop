# App-owned native components

An app can ship its own native UI without rebuilding NativeDesktop. The prebuilt host loads
app-owned shared libraries at launch, and the generic `NativeView` widget routes lifecycle
operations to a factory that library registers.

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

`props` is JSON-serialized. A ref exposes `send(command, arg)`, or you can call
`sendNativeCommand(ref.current, command, arg)`. Native events arrive as `{ name, data }`.
`NativeView` is an opaque leaf, so React children cannot mount inside it.

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

`nd dev` and `nd build` run these app-owned commands, and only when the inputs are newer than the
output. Neither ever invokes `zig build` or rebuilds the host. The resulting paths are passed through
`ND_PLUGIN_PATHS`, and the older `ND_PLUGIN_PATH` still works.

Build commands run with `ND_NATIVE_PACKAGE` set to the installed `@nativedesktop/native` package
root. On macOS the child environment also drops `SDKROOT` and the Nix compiler variables and sets
`DEVELOPER_DIR` from `xcode-select -p`, so a plain `xcrun swiftc` finds the real Xcode toolchain.

`inputs` are literal paths resolved from the app directory with no environment expansion, so
reference the package as `node_modules/@nativedesktop/native`.

## ABI

Use `@nativedesktop/native/include/nd_plugin.h`. Export `nd_plugin_entry()` and declare ABI v3. Register an `nd_view_impl` in `init()`:

- `create(props_json)` returns `GtkWidget*` on Linux or `NSView*` on macOS.
- `apply_props` handles later React props.
- `connect(view, node_id)` records the node identity.
- `command` handles imperative commands.
- `destroy` releases app-owned state exactly once.
- `registry->emit_event(registry, node_id, name, payload_json)` emits an event.

The registry pointer and its callbacks stay valid for the lifetime of the loaded plugin. Any call
that touches widgets happens on the platform UI thread. ABI v1 command plugins and ABI v2 view
plugins still load.

## macOS

Return an AppKit `NSView`. `-I "$ND_NATIVE_PACKAGE/macos"` provides the `CNdPlugin` Clang module,
which is the C ABI header, and compiling `$ND_NATIVE_PACKAGE/macos/NativeDesktopNative.swift` into
the dylib provides the Swift helpers. Conform to `NativeDesktopView`, return
`NativeDesktopPlugin.descriptor(...)` from `nd_plugin_entry`, and call
`NativeDesktopPlugin.registerView` from its initialize callback. That path supports one view kind per
plugin; hand-roll an `nd_view_impl` to register more.

For SwiftUI, `NativeDesktopSwiftUIView` hosts a SwiftUI tree behind the same protocol. It follows the
ownership rule the framework's SettingsGroup bridge uses: NativeDesktop retains one AppKit identity
while SwiftUI controls the content and placement inside it.

Production `.app` packaging has to embed the app dylib and include it in signing and notarization.
The development loader accepts an absolute build path.

## Linux

Build a `.so` against GTK 4 and the stable C header. The app developer's machine needs GTK
development headers and `pkg-config gtk4`. `nd_native_gtk.h`, reachable through
`-I "$ND_NATIVE_PACKAGE/linux"`, carries `nd_gtk_view_state`, `nd_gtk_connect_state`, and
`nd_gtk_emit` for per-view registry and node bookkeeping. Allocate and free component state in the
app library rather than relying on framework allocators.

`examples/nativeview-demo` has working GTK and AppKit implementations, typed React props, an event,
prop updates, the config, and cached build commands.
