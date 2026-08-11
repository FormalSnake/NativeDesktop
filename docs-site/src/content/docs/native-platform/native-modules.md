---
title: Native Modules
description: Ship app-owned GTK/AppKit widgets NativeDesktop doesn't provide, loaded as a native plugin shared library and rendered through the generic <nativeview> widget.
---

The built-in widget set covers common UI. When an app needs something it doesn't cover (a map
view, a chart, a platform SDK's own view, a piece of legacy AppKit/GTK code), it can ship its own
native widget as a plugin, without rebuilding NativeDesktop itself. The host loads an app-owned
shared library at launch, and the generic `<nativeview>` widget routes create/update/command/event
traffic to a factory that library registers.

## When to reach for this

Reach for a native module when the thing you need is a real platform view with its own drawing,
input handling, or SDK, not a layout of existing widgets. Prefer composing `<box>`, `<table>`,
`<image>`, and friends whenever possible: a native module is opaque to React (no children mount
inside it), costs you two platform-specific implementations, and moves outside the schema's
compile-time guarantees. Reach for the escape hatch, not the default.

## React API

```tsx
import { defineNativeComponent, type NativeComponentRef } from "@nativedesktop/react";

interface ColorProps { color: string }
interface ColorEvent { source: "gtk" | "appkit" }

const ColorView = defineNativeComponent<ColorProps, ColorEvent>({ viewKind: "app.colorview" });

<ColorView
  props={{ color: "#3b82f6" }}
  onNativeEvent={({ name, data }) => {
    if (name === "pressed") console.log("pressed from", data.source);
  }}
/>;
```

`defineNativeComponent` wraps the `<nativeview>` intrinsic in a typed component: `viewKind`
identifies which factory the plugin registered, `props` is JSON-serialized across the ABI, and
`onNativeEvent` receives `{ name, data }` as the plugin emits them. A ref exposes
`send(command, arg?)` for one-shot imperative calls, or use `sendNativeCommand(ref.current,
command, arg)` directly from `@nativedesktop/react`. This is a sibling channel to the
schema-typed `sendCommand`/`hasCommand` covered in
[Imperative Commands & Refs](/core-concepts/imperative-commands/): `<nativeview>` declares no
`commands` in `schema/widgets.json` (it has none to validate against), so `sendNativeCommand`
skips that validation and hands the command straight to the plugin's own `command` handler.

## `<nativeview>` widget

Automation role: `custom`. Text source: none. Children: none (an opaque leaf).

| Prop | Type | Default | Applied |
| --- | --- | --- | --- |
| `viewKind` | string | none | create |
| `props` | string | `{}` | createAndUpdate |
| `testID` | string | none | meta |

| Event | Handler | Payload |
| --- | --- | --- |
| `nativeEvent` | `onNativeEvent` | data |

You will rarely author `<nativeview>` directly: `defineNativeComponent` generates it with
`viewKind` fixed and `props` JSON-stringified for you.

## Configuration and builds

Declare `@nativedesktop/native` as a dependency and add a `native.plugins` entry to
`nativedesktop.config.ts`:

```ts
import { defineConfig } from "@nativedesktop/cli/config";

export default defineConfig({
  native: {
    plugins: [{
      darwin: "native/build/libcolorview.dylib",
      linux: "native/build/libcolorview.so",
      build: {
        darwin: {
          command: ["bash", "-c",
            "mkdir -p native/build && /usr/bin/xcrun --sdk macosx swiftc -emit-library " +
            "-o native/build/libcolorview.dylib -I \"$ND_NATIVE_PACKAGE/macos\" " +
            "native/macos/ColorView.swift \"$ND_NATIVE_PACKAGE/macos/NativeDesktopNative.swift\" " +
            "-framework AppKit -framework SwiftUI"],
          inputs: ["native/macos", "node_modules/@nativedesktop/native"],
        },
        linux: {
          command: ["bash", "-c",
            "mkdir -p native/build && cc -shared -fPIC $(pkg-config --cflags gtk4) " +
            "-I \"$ND_NATIVE_PACKAGE/include\" -I \"$ND_NATIVE_PACKAGE/linux\" " +
            "native/linux/colorview.c -o native/build/libcolorview.so $(pkg-config --libs gtk4)"],
          inputs: ["native/linux", "node_modules/@nativedesktop/native"],
        },
      },
    }],
  },
});
```

`nd dev` and `nd build` run each platform's `build.command` and only when its `inputs` are newer
than the declared output. They never invoke `zig build` or rebuild the host itself. Build commands
run with `ND_NATIVE_PACKAGE` set to the installed `@nativedesktop/native` package root; on macOS the
child environment also drops `SDKROOT` and the Nix compiler variables and sets `DEVELOPER_DIR` from
`xcode-select -p`, so a plain `xcrun swiftc` resolves the real Xcode toolchain instead of a Nix one.
`inputs` are literal paths resolved from the app directory with no environment expansion, so
reference the package as `node_modules/@nativedesktop/native`.

## Loading contract: `ND_PLUGINS` / `ND_PLUGIN_PATHS`

The resolved plugin output paths for the current platform are passed to the host through
`ND_PLUGIN_PATHS` (colon-separated; the older single-path `ND_PLUGIN_PATH` still works), gated by
`ND_PLUGINS=1`. `nd dev`/`nd build` set both automatically from `native.plugins`; a packaged app's
launch script sets them to the bundled plugin paths under the app root. On startup, once
`ND_PLUGINS=1` is set, the host calls `nd_load_plugins_from_env`, which splits `ND_PLUGIN_PATHS` on
`:`, skips empty segments, and calls `nd_load_plugin` on each path in order. Nothing loads a plugin
unless the embedder explicitly opts in this way. There's no scanning of a plugins directory.

## The ABI

The contract is `@nativedesktop/native/include/nd_plugin.h`, a plain C header, append-only across
versions (the current version is `ND_PLUGIN_ABI_VERSION 3`; v1/v2 plugins still load, since new
fields only ever get appended, never removed or reordered). A plugin is a shared library exporting
one symbol:

```c
const nd_plugin_v1* nd_plugin_entry(void);
```

`nd_plugin_v1` declares `abi_version`, a `name`, a `NULL`-terminated `capabilities` list (permission
strings like `"plugin:hello.greet"`, checked against the app's ACL grants before a registered
command runs), an `init(registry)` that registers commands/views, and a `deinit()`. To back a widget
kind, `init` calls the registry's `register_view(registry, view_kind, &nd_view_impl)` with:

- `create(props_json)`: returns the native widget as an opaque pointer (`GtkWidget*` on Linux,
  `NSView*` on macOS). The core never dereferences it; it only moves the pointer through
  append/unparent by parent kind.
- `apply_props(view, props_json)`: handles later React prop updates.
- `command(view, command, arg_json)`: handles `sendNativeCommand` calls.
- `destroy(view)`: releases app-owned state exactly once.
- `connect(view, node_id)`: (ABI v3) records the node's identity so the view can emit events.

Once connected, the plugin calls `registry->emit_event(registry, node_id, name, payload_json)` to
send a `nativeEvent` back to React. The registry pointer and its callbacks stay valid for the
lifetime of the loaded plugin, and any call that touches a widget happens on the platform UI thread.
Adding a capability to this ABI means appending a field at the end of the struct and bumping
`ND_PLUGIN_ABI_VERSION`, never reordering or removing one: the same append-only discipline the
core's `nd_backend` vtable follows.

## Minimal example

`examples/nativeview-demo` is a complete, working plugin: a `ColorView` that renders a solid color
box, forwards a `color` prop, emits a `pressed` event on click, and answers a `reset` command.

```tsx
// examples/nativeview-demo/main.tsx
import { defineNativeComponent, render, useRef, useState, type NativeComponentRef } from "@nativedesktop/react";

interface ColorProps { color: string }
interface ColorEvent { source: "gtk" | "appkit" }
type ColorCommand = Record<string, never>;

const ColorView = defineNativeComponent<ColorProps, ColorEvent, ColorCommand>({ viewKind: "app.colorview" });

function App(): React.ReactNode {
  const [color, setColor] = useState("#3b82f6");
  const [lastSource, setLastSource] = useState("none");
  const native = useRef<NativeComponentRef>(null);
  return (
    <window title="App-owned Native Component" defaultWidth={480} defaultHeight={360}>
      <box orientation="vertical" spacing={12}>
        <ColorView
          ref={native}
          props={{ color }}
          onNativeEvent={({ name, data }) => {
            if (name === "pressed") setLastSource(data.source);
          }}
          style={{ hexpand: true, vexpand: true }}
        />
        <label text={`Native event source: ${lastSource}`} />
        <box orientation="horizontal" spacing={8}>
          <button label="Change color" onClick={() => setColor(color === "#3b82f6" ? "#ef4444" : "#3b82f6")} />
          <button label="Reset natively" onClick={() => native.current?.send("reset", {})} />
        </box>
      </box>
    </window>
  );
}

await render(<App />);
```

Its `native/linux/colorview.c` and `native/macos/ColorView.swift` implement the `nd_view_impl` and
`NativeDesktopView` sides, and its `nativedesktop.config.ts` is the config block shown above with
the paths and build commands filled in.

## Platform notes

**macOS**: return an `NSView`. `-I "$ND_NATIVE_PACKAGE/macos"` provides the `CNdPlugin` Clang
module (the C ABI header), and compiling `$ND_NATIVE_PACKAGE/macos/NativeDesktopNative.swift` into
the plugin dylib provides Swift helpers. Conform to `NativeDesktopView`, return
`NativeDesktopPlugin.descriptor(...)` from `nd_plugin_entry`, and call
`NativeDesktopPlugin.registerView` from `init`; that path supports one view kind per plugin; hand-roll
an `nd_view_impl` to register more. `NativeDesktopSwiftUIView` hosts a SwiftUI tree behind the same
protocol, following the same ownership rule as the framework's own `SettingsGroup` bridge:
NativeDesktop retains one AppKit identity while SwiftUI controls the content inside it. Production `.app` packaging
has to embed the plugin dylib and include it in signing and notarization; the development loader
accepts an absolute build path.

**Linux**: build a `.so` against GTK 4 and the stable C header. The developer's machine needs GTK
development headers and `pkg-config gtk4`. `nd_native_gtk.h`, reachable through
`-I "$ND_NATIVE_PACKAGE/linux"`, carries `nd_gtk_view_state`, `nd_gtk_connect_state`, and
`nd_gtk_emit` for per-view registry and node bookkeeping. Allocate and free component state in the
plugin library itself rather than relying on framework allocators.

See the [Widget Reference](/components/widget-reference/) for `<nativeview>`'s generated prop
table alongside every other widget, and
[Imperative Commands & Refs](/core-concepts/imperative-commands/) for how `sendCommand` and
`sendNativeCommand` relate.
