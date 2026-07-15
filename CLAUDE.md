# CLAUDE.md

Project-level guidance for NativeDesktop — a cross-platform native UI toolkit
where one React/TSX codebase renders to real native widgets (libadwaita/GTK on
Linux, AppKit on macOS).

## Fix UI at the framework level, not the app level

When asked to fix how the UI looks or feels on a platform — spacing, native
chrome, sidebar/selection styling, control metrics, anything that reads as
"not native" — the fix belongs in the **platform backend** (`swift/Sources/
NDShell`, `swift/Sources/NDGen`, or the Zig/GTK side), so the **same, unchanged
app tree** renders natively. Do NOT reach for per-app pixel-tuning in an example
or app (hand-set `padding`, one-off `cssClasses`, magic offsets).

The toolkit's whole promise is **one codebase → native on every platform**.
App-level tuning breaks that promise: it fixes one screen, doesn't generalize,
and drifts from native as the OS evolves. A structural class like
`navigation-sidebar` should carry real native semantics on each backend, not be
a no-op that apps then paper over with manual styling.

Platforms may legitimately look *different from each other* — accept that, as
long as each one looks native **for its own OS**. Match the platform, not the
other platform.

Fall back to changing app/example code only when the tree genuinely cannot
express the intent — and when you do, say so explicitly rather than editing
silently.

## Architecture (mental model for fast onboarding)

**Two processes per app.** A native **host** (Zig, owns `main()` + the OS event
loop + the authoritative retained widget tree) and a **Bun/TypeScript child**
(runs your React app). React's reconciler diffs the tree and sends
`CommitBatch`es to the host over **NDP** — length-prefixed JSON over a local
socket; events flow back the same pipe. A JS crash or hang does not take the
window down — the host stays up.

**One core, two backends, one seam.** The core is `libnd.a` — pure Zig,
GTK-free *and* AppKit-free: protocol, reconciler (`src/tree.zig`), runtime
(`src/runtime.zig`), ACL (`src/acl.zig`), and the C-ABI vtable seam
(`src/abi.zig`, `src/abi_backend.zig`). Both backends register the identical
`nd_backend` vtable via `nd_register_backend`; widgets cross as opaque handles
(`GtkWidget*` / `NSView*`) the core never dereferences. The vtable is
**append-only** — append new ops at the END, never reorder or remove. There is
no ABI version *constant* ("ABI v3" in git is a milestone label); the guard is a
compile-time layout assert (`@sizeOf(NdBackend) == N * @sizeOf(usize)`) plus
`scripts/sync-native-headers.sh`, which mirrors the header into
`packages/native/include/` (CI cmp-checks it). Adding an op means: append it,
bump the assert, run the sync script.

**Why GTK looks "built-in" but AppKit looks "separate" — it's packaging, not
architecture.** GTK is a C library, so Zig drives it natively and the GTK
embedder (`src/gtk/`) compiles *into* the Linux host binary. AppKit is a
Swift/ObjC framework you can't idiomatically drive from Zig, so its backend is a
separate SwiftPM host (`swift/Sources/NDShell/`) that links `libnd.a` as a
static archive. Both are equal peers at the seam; only the host language and
build system differ, because each platform's native UI lives in a different
language.

**The Bun child is a full runtime, not a sandboxed renderer.** Unlike Electron's
renderer, the app process has `node:fs`, `bun:sqlite`, process spawning, and
network — no `contextBridge`/IPC. "Backend" logic lives in the same process as
the UI. Consequence: heavy *synchronous* work on the Bun main thread stalls
React's commit loop (UI updates pause), but NOT the native window or an embedded
webview — those render in the separate host process (web content in WebKit's own
processes). Use `@nativedesktop/data` (below) to keep the main thread free.

**Schemas are the single source of truth**, fed through `tools/codegen.ts`:
`schema/widgets.json` (widget props/events/commands/automation role),
`schema/protocol.json` (NDP frames), `schema/rpc.json` (automation RPC).
Renaming a field is a compile error on both the Zig and TS sides. Never
hand-edit `tools/codegen.ts` or the generated files.

**Windowing:** multi-window works — render multiple `<window>` roots (e.g. a
fragment) and each opens an independent OS window on both backends. The core
reconciler (`src/tree.zig`) pools window handles by node id; the ABI's
`resolve_window` op rebinds windows on crash/HMR respawn. Because all windows
live in one Bun/React process, cross-window state sync is trivial — shared
state, no IPC. Per-window scoping is now done: automation measures each widget
against its OWN window (so `getTree` bounds, `click`, `setValue`, `type`,
`scroll`, `waitFor` are per-window-correct), `screenshot` targets the requested
`window` (via the existing `resolve_window` op — no new ABI op), the crash
overlay paints on every open window, each toolbar attaches to its owning window,
and `core:window.create` is ACL-gated per window id. One limitation remains:
`getTree` has no `window` param yet, so it returns the root window's tree with
other windows' nodes attached as orphans.

**Cross-window reparenting (drag a tab between windows without reload):** React
can't express a widget-preserving cross-parent move — moving a node to a new
parent unmounts+remounts it, destroying the native widget (a `<webview>` would
reload). Solution: render the movable node via `createPortal(node, pool)` into a
process-lifetime **pool** so React never unmounts it, then relocate the *live
native widget* imperatively with `moveNode(ref, slotRef)`. That rides the
existing `widgetCommand` frame (reserved `__ndReparent`) into the appended
`reparent_child` ABI op (vtable now 21 words); GTK brackets the move in
`g_object_ref`/`unref`, AppKit relies on the core's `passRetained` +1. See
`examples/multiwindow/`. `moveNode` is imperative BY DESIGN — preserving state
React would otherwise destroy is outside `UI = f(state)`.

**Webview / browser-style apps:** `<webview>` wraps the platform engine
(WKWebView / WebKitGTK) with a browser-grade surface — full docs in
`docs/webview.md`. Prop `url`; events `navigate`, `titleChanged`,
`loadingChanged`, `backAvailable`, `forwardAvailable`, `loadProgress`,
`loadFailed`, `newWindow` (host denies the popup, app opens a native tab),
`downloadRequested` (engine download cancelled, app downloads via Bun),
`javaScriptResult`; commands `goBack`, `goForward`, `reload`, `stop`,
`executeJavaScript` (promise helper in `@nativedesktop/react`), `setZoom`,
`setUserAgent`, `openDevTools`. Adding a WebView *event* needs one-line entries
in codegen's `SIGNALS` + `SWIFT_SIGNALS` tables; commands need schema only.
Request interception/user scripts are not exposed yet. CEF is a planned opt-in
engine (per project, per platform) — never linked into the host, loaded at
runtime, and packaging must ship zero Chromium bytes unless the app's config
enables it (and `strip` libcef.so on Linux when it does).

**Keyboard shortcuts:** menu accelerators are native (an `accelerator` prop →
`keyEquivalent` + modifiers on macOS, `MenuBar.swift`). There is no general
widget-level `onKeyDown` yet.

### App-facing APIs added recently
- **System capabilities** (parity with native-sdk.dev's capability pack) —
  promise-based APIs from `@nativedesktop/react` (`packages/react/src/
  system.ts`): `dialog.openFile/saveFile/showMessage`, `clipboard.readText/
  writeText`, `notifications.show/onClick`, `recentDocuments.add/clear`,
  `credentials.set/get/delete` (Keychain / dlopen'd libsecret), and app-level
  event subscriptions `app.onActivate/onDeactivate/onOpenUrl/onOpenFile/
  onFileDrop` (each returns an unsubscribe fn). Wire: `systemRequest` /
  `systemResponse` (id-correlated) + `systemEvent` NDP frames → one coarse
  `system_request` vtable op (#22); backends reply async via
  `nd_system_response` / push via `nd_system_event`. ACL-gated per method
  group: `core:dialog`, `core:notification`, `core:recent`,
  `core:clipboard.write`, `core:audio` are default-granted;
  `core:clipboard.read` and `core:credentials` need an `ND_ACL_GRANTS`
  manifest (`{"defaultWindow":[...]}` or per-window `grants`). Pure-TS
  helpers (no host round-trip, unsandboxed by design): `openExternal` /
  `openPath` / `revealPath` (`packages/react/src/shell.ts`).
- **Audio playback + spectrum** — `audio.play({path|url, volume?, spectrum?})`
  → handle; `pause/resume/stop/seek/setVolume`; `audio.onState` (transition
  events: playing/paused/ended/stopped/error, position/duration ms) and
  `audio.onSpectrum` (32 log-spaced bins 0..1, ~15 Hz, opt-in per handle).
  macOS: AVPlayer + MTAudioProcessingTap + vDSP FFT
  (`swift/Sources/NDShell/Audio.swift`); Linux: GStreamer playbin + spectrum
  element, dlopen'd per the no-link rule (`src/gtk/audio.zig`), degrades to
  "audio unavailable" without GStreamer. Rides systemRequest/systemEvent —
  no ABI/protocol additions.
  Known gaps across system APIs: `fileDrop.windowId` is always 0;
  `notification.click` needs a bundled app on macOS (bare NDShell uses the
  legacy NSUserNotification path); no host-side buffering of
  `app.openUrl`/`openFile` delivered before the child connects.
- **File associations + URL schemes** — `app: { id, fileAssociations,
  urlSchemes }` in `nativedesktop.config.ts` (`packages/nd/src/config.ts`),
  injected at package time into Info.plist (`CFBundleDocumentTypes` /
  `CFBundleURLTypes`) and the Linux `.desktop` `MimeType=` + shared-mime-info
  XML (`tools/app-identity.ts`). OS launches land as `app.onOpenFile` /
  `app.onOpenUrl` events.
- **App data dir** — `getAppDataDir()` / `ensureAppDataDir()` from
  `@nativedesktop/react` (`packages/react/src/paths.ts`). Electron-style
  userData path; app name comes from the app's `package.json` `name`. macOS
  `~/Library/Application Support/<name>`, Linux `$XDG_DATA_HOME/<name>`
  (→ `~/.local/share/<name>`).
- **Worker-backed SQLite** — `@nativedesktop/data` (`packages/data/`).
  `openDatabase(path)` → `query` / `mutate` / `transaction` / `close`, plus a
  `useQuery` hook from `@nativedesktop/data/react`. The `bun:sqlite` connection
  runs in a Bun `Worker`, so queries never block React's commit loop. Composes
  with `ensureAppDataDir()`. **ORM-agnostic by design:** the library depends on
  NO ORM; it exports a stable `SqliteExecutor` contract (async
  `query`/`mutate`/`transaction`) that adapters target in userland. Worked,
  tested examples in `packages/data/src/adapters.test.ts`: Drizzle via
  `drizzle-orm/sqlite-proxy` (async callback → `exec`; bridge rows with
  `Object.values`, alias duplicate-named join columns) and Kysely via a custom
  `Dialect` (async-native, no conversion — the cleaner fit). Raw SQL stays
  first-class. Migrations: drizzle-kit unchanged (own process against the file)
  or `migrate()` at startup via the sqlite-proxy migrator. `drizzle-orm`/`kysely`
  are devDependencies only — never a library dep to keep updated.
- **Per-window dialogs + toasts** — `showAlert`/`openFile`/`saveFile`/`showAbout`
  (`packages/react/src/dialogs.ts`) and `showToast`/`dismissToast`
  (`packages/react/src/toast.ts`) are promise-wrapped imperative commands on
  `<window>`/`<toastoverlay>`: `sendCommand` kicks the native dialog/toast off,
  a matching `*Result`/`toast*` event settles the promise. Dialogs correlate by
  the window's own wire id (only one pending per window — a second call
  rejects immediately; `showAbout` has no result event so it doesn't claim the
  slot); toasts correlate by a generated `id` so the overlay can queue several.
  Distinct from `system.ts`'s ACL-gated `dialog.*` above — that one is
  app-level, not window-scoped, and has no About panel. Docs:
  `docs-site/src/content/docs/components/dialogs.md` and `.../feedback.md`.
- **Platform-only widgets** — a schema `platforms: ["macos"]` list
  (`Widget.platforms` in `tools/codegen.ts`) permanently no-ops a widget's
  create/apply/structural arms on excluded backends (`ND_PLATFORM_NOOP` —
  distinct from the temporary stub registry below) and is exported to JS as
  `schema-meta.ts`'s `widgetPlatforms`. `host-config.ts`'s `checkPlatform` logs
  a one-time `console.warn` in `nd dev` (`isHot()`-gated, absent from `nd
  build`) when such a widget mounts on an excluded platform. Gate app code with
  `Platform.os` (an OS-level API like `NSStatusItem` exists or doesn't),
  never `Platform.backend` (that's a renderer difference). Today's two:
  `<trayitem>`, `<sharebutton>`.
- **Table/TreeView data conventions** — `<table>` takes `columns:
  TableColumn[]` (`{id,title,width?}`) and `rows: TableRow[]` (`{id?,cells:
  string[]}`, cells positional by column order); header-click sorting fires
  `onSortChanged({columnId,direction})` and the native widget never reorders
  `rows` itself — the app re-sorts and feeds the new array back down.
  `<treeview>` takes a **flat** `nodes: TreeNode[]` (`{id,parentId?,title,
  badge?,iconName?,hasChildren,expanded}`, root nodes omit `parentId`);
  `onNodeExpanded`/`onNodeCollapsed` fire `{nodeId}` and expansion is
  app-controlled state, never native state, so an unrelated re-render can't
  silently collapse a branch the user opened.
- **Codegen per-widget template protocol** — every widget needs a
  create/applyProps/signal template on both Zig and Swift (containers also
  need a structural attach/detach template); one missing makes
  `tools/codegen.ts` throw at generation time rather than drift silently.
  `ZIG_STUB_WIDGETS`/`SWIFT_STUB_WIDGETS` (top of `tools/codegen.ts`) name
  widgets still mid-implementation on one backend — they get a `ND_STUB(Name)`
  placeholder arm instead of the throw; both sets are empty as of M15 (every
  widget now has real templates on both backends). Platform-excluded widgets
  never use this registry — they get the permanent `ND_PLATFORM_NOOP` arms
  above instead, which is a different mechanism from a stub-to-be-filled.

### File map
- **Core (Zig):** `src/tree.zig` (reconciler), `src/abi.zig` +
  `src/abi_backend.zig` (ABI seam), `src/runtime.zig`, `src/protocol.zig`,
  `src/acl.zig`, `src/backend.zig`.
- **GTK backend:** `src/gtk/` — `main.zig`, `backend.zig`, `webview.zig`,
  `style.zig`, `terminal.zig`.
- **AppKit backend:** `swift/Sources/NDShell/` (hand-written) +
  `swift/Sources/NDGen/` (generated) + `swift/Sources/CNd/` (bridges `libnd.a`);
  build scripts under `scripts/mac/`.
- **JS packages:** `packages/react` (`@nativedesktop/react`: reconciler,
  intrinsics, `Platform`, paths), `packages/host` (resolves the prebuilt host
  binary), `packages/nd` (the `nd` CLI — `nd dev` / `nd build`), `packages/data`
  (worker SQLite).
- **Schemas:** `schema/{widgets,protocol,rpc}.json` → `tools/codegen.ts` →
  generated Zig/TS/Swift.
- **Build:** `zig build` → `zig-out/bin/nd-hello` (Zig host); the SwiftPM
  package in `swift/` builds the AppKit shell; `bun run dev` == `nd dev`.

NEVER EDIT tools/codegen.ts ON YOUR OWN! JUST DELETE THE FILE IT SHOULD AUTO GENERATE WITH THE COMMAND
