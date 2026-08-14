# NativeDesktop

Write React 19 in TypeScript, get real native desktop widgets: GTK4 and libadwaita on Linux, AppKit
on macOS. No DOM, no Electron, no browser on the UI path.

The widgets your JSX describes are the platform's own classes (`GtkBox`, `AdwHeaderBar`, `NSButton`,
`NSSplitView`), so one React tree renders in each platform's current design language rather than a
lookalike layer approximating both.

Every app is two processes. A native host owns `main()`, the OS event loop, and the authoritative
widget tree; a Bun child runs your React code. The reconciler ships each commit over a local socket
and events come back the same way, so a JS crash or hang leaves the window standing and the host
restarts the child. The child is a full Bun runtime (`node:fs`, `bun:sqlite`, subprocesses, network
access), so app logic lives in the same process as the UI with no IPC bridge.

```bash
bun add @nativedesktop/cli @nativedesktop/react react
bunx nd dev src/main.tsx
```

```tsx
// src/main.tsx
import { render, useState } from "@nativedesktop/react";

function App() {
  const [clicks, setClicks] = useState(0);

  return (
    <window title="Counter" defaultWidth={480} defaultHeight={320}>
      <box orientation="vertical" spacing={8}>
        <label text={`Clicks: ${clicks}`} />
        <button label="Increment" onClick={() => setClicks((c) => c + 1)} />
      </box>
    </window>
  );
}

await render(<App />);
```

Hooks are imported from `@nativedesktop/react`, not from `react`: hot reload re-evaluates the whole
module graph, and a bare `react` import would resolve to a fresh instance with no attached
dispatcher. Shared-logic packages still write against `react`; the build rewrites the import. See
[State & Hot Reload](docs-site/src/content/docs/core-concepts/state-hot-reload.md).

Host binaries ship prebuilt for macOS on Apple silicon and x86_64 Linux, pulled in as
`optionalDependencies` the way Electron and esbuild do it.

## Working from a checkout

To hack on the framework itself, or to run on a platform with no prebuilt host, you need Zig 0.16.0
and Bun 1.3.13. `nix develop` pins both. On macOS, nixpkgs' libadwaita fails to build, so run
`brew install libadwaita` once instead (it pulls in gtk4).

```bash
git clone https://github.com/FormalSnake/NativeDesktop
cd NativeDesktop && bun install
cd examples/counter && bun run dev
```

`bun run dev` is `nd dev`: it resolves the host binary through `@nativedesktop/host`, builds it on
first run inside this checkout, and starts with hot reload and the crash-restart overlay on. macOS
runs the AppKit shell, Linux the GTK host; `bun run dev -- --backend gtk` cross-checks the GTK
backend on macOS through its Quartz gdk. Scaffold your own app against the checkout, with
dependencies rewritten to `file:` paths:

```bash
./scripts/new-app.sh ../my-app
cd ../my-app && bun install && bun run dev
```

## What's in the box

- **57 widgets** generated from one schema: layout, forms, `<headerbar>`/`<splitview>` chrome,
  `<table>` and `<treeview>` data views, a `<sourcetree>` sidebar, `<paned>` splits, a
  `<commandpalette>`, a `<terminal>` backed by libghostty-vt, and a `<webview>` (WKWebView,
  WebKitGTK) with navigation events, downloads, popup handling, and `executeJavaScript`.
  Reference: [docs/widgets.md](docs/widgets.md).
- **Multiple windows and native tabs.** Several `<window>` roots in one React process share state
  with no IPC; `tabGroup` joins them into the platform's own tab system, and a `<webview>` tab
  dragged to another window survives without reloading. See
  [Native Tabs](docs-site/src/content/docs/native-platform/tabs.md).
- **System APIs**, ACL-gated per method group: file dialogs, clipboard, notifications, recent
  documents, Keychain and libsecret credentials, audio playback with a spectrum feed, and app
  events like `onOpenUrl` and `onFileDrop`. See
  [System Capabilities](docs-site/src/content/docs/native-platform/system-capabilities.md).
- **Packages**: `@nativedesktop/react` (the renderer), `data` (SQLite in a Bun `Worker` so queries
  never block the commit loop, ORM-agnostic), `rpc` (typed client with reconnect backoff over
  socket or WebSocket), `panes` (splittable pane-tree state and component), `test` (automation
  harness), `native` (app-owned GTK/AppKit plugin widgets, no framework rebuild).
- **The `nd` CLI**: `nd dev` (hot reload plus crash overlay), `nd build` (Babel with a React
  Compiler pass), `nd package mac|linux` (signed `.app`, AppImage), `nd doctor` (packaging and
  toolchain readiness checks).
- **Built for coding agents.** With `NATIVE_AUTOMATION=1` the host serves a JSON-RPC socket:
  `getTree` returns an accessibility tree with roles and live values, and on macOS `pointer`,
  `drag`, and `keys` post real NSEvents. `@nativedesktop/test` wraps it in `launchApp`, node
  queries, waits, and screenshots, so an agent drives the app the way a person does. See
  [Automation Socket](docs-site/src/content/docs/automation-testing/automation-socket.md).
- **Opt-in updater**: `nd package` with an `updates` config emits a minisign-signed archive and
  manifest that the host verifies before applying. See [docs/packaging.md](docs/packaging.md).

## Requirements

macOS 15 or newer on Apple silicon; the shell links only system frameworks. On x86_64 Linux the
floor is libadwaita 1.7 with the glibc and GTK versions of the ubuntu:26.04 build container, linked
dynamically at run time. Details and per-distro package names:
[docs/runtime-deps.md](docs/runtime-deps.md).

## Status

v0.1.1, early. Prebuilt hosts cover macOS arm64 and Linux x64. Linux has a blocking CI gate (unit
tests, codegen freshness, headless drive scripts under Weston); macOS runs its drive scripts as a
non-blocking job. Windows is designed but not implemented, and `nd package` rejects it as a target.
Mobile is out of scope. See
[Platform Support](docs-site/src/content/docs/native-platform/platform-support.md).

## Docs

The full site lives in [`docs-site/`](docs-site); `cd docs-site && bun install && bun run dev`
serves it locally. Start with
[Introduction](docs-site/src/content/docs/get-started/introduction.md) and
[Quick Start](docs-site/src/content/docs/get-started/quick-start.md), which also covers building
the Zig host and the Swift shell by hand.

Contributions are welcome; open an issue before starting anything large.

## License

MIT, see [LICENSE](LICENSE). Third-party attributions, including the vendored libghostty-vt
archives and the dynamically linked LGPL GTK stack on Linux, are in [NOTICE](NOTICE).
