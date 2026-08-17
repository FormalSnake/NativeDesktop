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
compile-time layout assert (`@sizeOf(NdBackend) == 23 * @sizeOf(usize)` today)
plus `scripts/sync-native-headers.sh`, which mirrors the header into
`packages/native/include/` (CI cmp-checks it). Adding an op means: append it,
bump the assert, run the sync script. Node ownership: each backend holds ONE
strong ref per tracked node (GTK `ref_sink` at create, AppKit `passRetained`),
dropped through the appended `release_node` op (word 23) when the core forgets
the id (remove arm, generation GC). Before that op, GTK handles were floating
refs and the first `getTree` could probe a freed GObject.

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
state, no IPC. Per-window scoping is done: automation measures each widget
against its OWN window, `screenshot` targets the requested `window` (existing
`resolve_window` op, no new ABI op), the crash overlay paints on every open
window, toolbars attach to their owning window, and `core:window.create` is
ACL-gated per window id. `getTree` takes an optional `window` param scoping the
snapshot to that window's subtree; absent, the root window's tree is returned
with other windows' nodes attached as orphans.

**Agentic testing (M16):** `getTree` is an accessibility tree — every node
carries `role` (schema-declared, on the wire via the generated widget table),
`enabled`, `focused`, and `value` (live per-node `"a11y"` probe through the
existing `semantic_action` vtable op #17 — NO new ABI op; backends without the
probe degrade to defaults). Input-synthesis RPCs `pointer`/`drag`/`keys`/
`doubleClick`/`rightClick`/`hover` post real NSEvents through the app's event
queue on macOS (drag posts the whole down/dragged/up batch in ONE marshal —
AppKit tracking loops block the main thread mid-gesture and would deadlock
per-phase marshaling); GTK answers `-32003 inputUnsupported` (GTK4 removed
app-constructible events) — semantic click/setValue/type/scroll remain the
Linux path. Acceptance: `scripts/mac/mac-gestures.sh` (examples/gestures +
gestures-drive.ts, keys-menu-drive.ts) → `MAC_GESTURES_OK`. Full surface doc:
`docs-site/src/content/docs/automation-testing/automation-socket.md`.

**Automation vocabulary + test harness:** `waitFor` runs host-side on the
retained tree (~50ms tick, never a getTree round-trip): exactly one selector of
`textContains` | `refVisible` | `testId`, and with `testId` a `state` of
present|gone|visible|enabled|disabled|focused plus `countAtLeast` /
`valueEquals` / `valueContains` refinements (compared against the a11y value's
STRING rendering, so one predicate fits TextInput and Slider alike). Every
targeting RPC (`click`/`setValue`/`type`/`scroll`/`doubleClick`/`rightClick`/
`hover`) takes exactly one of `ref`/`testId`; `resolve` ranks a testID's matches
(actionable first, key window first, then tree order); `windows` lists each
Window node's live key/main/visible/title plus `tabGroup`. Dialog scripting:
`ND_AUTOMATION_DIALOG_SCRIPT` (per-method FIFO JSON, `src/automation_dialogs.zig`)
answers `dialog.openFile/saveFile/showMessage` and
`window.showAlert/openFile/saveFile` with the REAL landed result shapes
(string[] / string|null / button index / dialogs.ts result objects), so drives
never block on a native dialog. `@nativedesktop/test` (`packages/test/`) is the
harness: `launchApp`/`AppHandle` spawn a host with `NATIVE_AUTOMATION=1`, parse
the stderr markers, and connect `socket.ts`'s `AutomationClient`, the repo's ONE
copy of the wire framing (packages/mcp and the drive scripts import it); plus
`poll` (for conditions the vocabulary can't express, e.g. window count),
`dialogScriptEnv`, `takeScreenshot`/`pngSize`. Screenshot: AppKit ghosting fixed
(stale cached bitmap reps); `ND_AUTOMATION_CAPTURE=screencapturekit` opts into a
ScreenCaptureKit rung.

**Native system tabs (M17):** `<window tabGroup="x">` roots render as one
tabbed window per platform's real tab system — macOS: each tab IS an NSWindow
joined via `addTabbedWindow` (identifier `nd.x`, `.preferred`; the tab bar's
`+` exists because `NDWindowTabDelegate` implements `newWindowForTab`); GTK:
the group owns scaffold AdwApplicationWindows (`AdwTabOverview{view}` >
`AdwTabView`, Ghostty's hierarchy) and each Window NODE's handle is an AdwBin
tab page — `src/gtk/tabs.zig` + `swift/Sources/NDShell/WindowTabs.swift`,
generated Window arms delegate. Framework injects GTK chrome (AdwTabBar with
a `+` end-action under the app's headerbar, AdwTabButton `overview.open`
packed at headerbar end). Events on the window node: `newTabRequested` (app
renders another `<window tabGroup>`), `closed` (app unmounts; the native
close is HELD PENDING until the unmount's remove op lands — AdwTabView's
async close-page contract / `windowShouldClose` false). JS-initiated unmount
closes natively via a `window.close` semantic action (tree.zig remove arm —
no new vtable op; plain windows ride it too, guarded no-op when already
closed; `isReleasedWhenClosed=false` at create makes close() safe). Tab
drag-out/in/reorder is native both sides; GTK `create-window` spawns a
sibling scaffold and the page (bin + child widget, webview state intact)
transfers; per-page chrome rebinds its `view` on `page-attached`;
`page-detached` must NEVER destroy or emit (fires on transfer). Cross-group
never merges (distinct identifiers). `showTabOverview` window command =
AdwTabOverview open / `toggleTabOverview`. `tabGroup` is create-only.
Overview's own new-tab button stays disabled on GTK BY DESIGN — its
`create-tab` signal demands a synchronously returned AdwTabPage, which the
async JS round-trip can't produce. Acceptance: `scripts/tabs-drive.ts`
(browser + terminal examples, cmd+t/cmd+w through real menu key equivalents)
→ `ND_TABS_OK`. Docs: `docs-site/src/content/docs/native-platform/tabs.md`.

**Cross-window reparenting (drag a tab between windows without reload):** React
can't express a widget-preserving cross-parent move — moving a node to a new
parent unmounts+remounts it, destroying the native widget (a `<webview>` would
reload). Solution: render the movable node via `createPortal(node, pool)` into a
process-lifetime **pool** so React never unmounts it, then relocate the *live
native widget* imperatively with `moveNode(ref, slotRef)`. That rides the
existing `widgetCommand` frame (reserved `__ndReparent`) into the appended
`reparent_child` ABI op (vtable word 21); GTK brackets the move in
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
`setUserAgent`, `openDevTools`. Context menus are the engine's own by default
(`contextMenuMode="native"`, the alternative being `"suppress"`): the app's
`setContextMenuItems` tree (types, checkboxes, radio groups, submenus,
per-context and per-target-URL filters) is merged into WebKit's menu in place,
and a chosen item arrives as `contextMenuItemClicked`. Adding a WebView *event*
needs one-line entries in codegen's `SIGNALS` + `SWIFT_SIGNALS` tables; commands
need schema only.
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
  XML (`packages/nd/src/package/identity.ts`). OS launches land as
  `app.onOpenFile` / `app.onOpenUrl` events.
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
- **Survivable error policy**: `setUnhandledErrorPolicy` / `onUnhandledError`
  (`packages/react/src/errors.ts`): `uncaughtException` defaults to fatal,
  `unhandledRejection` to report-and-survive (`ND_FATAL_REJECTIONS=1` flips
  it); render-phase errors route through the reconciler's createContainer
  callbacks. Non-fatal reports are rate-capped (20 per 10s) so an error loop
  can't saturate the NDP outbox. The `runtimeError` frame carries `fatal`: the
  host stashes overlay text only for fatal=true and prints
  `ND_RUNTIME_ERROR_NONFATAL` otherwise, so a stale report never becomes
  overlay text.
- **Settings store**: `createStore` / `useStoreValue`
  (`packages/react/src/store.ts`): versioned `${name}.json` under the app data
  dir. Intended launch shape: `await store.load()` above `render()`, which
  makes `get()` synchronous inside components (no loading flash, no restore
  effect). Writes are debounced, serialized on one promise chain, land via
  tmp+rename, with a synchronous last-resort flush on exit/SIGINT/SIGTERM.
  `migrate` runs on EVERY load (validate and upgrade in one hook; return null
  to reset); a corrupt file is backed up, defaults win, `loadError` is set.
- **`@nativedesktop/rpc`** (`packages/rpc/`): resilient JSON-RPC client for an
  app's own external services: typed `RpcContract`, `socketTransport` /
  `webSocketTransport`, `ConnectionLadder` reconnect backoff with a stability
  window. React binding lives at `@nativedesktop/rpc/react` (`useRpcStatus`
  subscribes to connection status) so the core client stays React-free;
  `@nativedesktop/react` is an optional peer dep.
- **`@nativedesktop/panes`** (`packages/panes/`): pure split-pane tree model
  over the existing `<paned>` widget (no schema/ABI change). Every model op
  (`splitPane`/`closePane`/`focusNeighbor`/`setPaneRatio`/`seedPanes`/
  `migratePanes`) returns the SAME reference when nothing changed, which stops
  a native positionChanged echo from looping a render+persist cycle.
  `PaneTree`/`usePaneTree` render the nested paneds; `renderLeaf` owns all
  per-pane chrome.
- **SourceTree sidebar**: `<sourcetree>`, a data-driven hierarchical sidebar
  (flat `id`/`parentId` nodes like `<treeview>`, controlled `selectedId` and
  expansion, per-row trailing `actions` with hover|always visibility, section
  group rows, badges, empty-state props). Events all carry `{nodeId}`
  (`actionClicked` adds `actionId`); no index payloads, since visible indexes
  are unstable across expand/collapse. macOS: `.sourceList` NSOutlineView that
  REUSES item instances by id so open branches survive React updates; GTK:
  `navigation-sidebar` GtkListBox of AdwActionRows, rebuilt per update. See
  `examples/sourcetree` + `scripts/sourcetree-drive.ts`.
- **HIG batch (GNOME + macOS 26/27 design language)**: new `<row>` /
  `<switchrow>` / `<clamp>` widgets (real AdwPreferencesGroup settings groups),
  ToolbarView top/content/bottom slots + bar styles, AdwViewSwitcher tab views,
  SplitView breakpoints + Window `sizeChanged`, empty-state AdwStatusPage on
  data views, `accentColor` in the appearance payload, and the spacing scale
  `Spacing`/`ContentMargin` (`packages/react/src/metrics.ts`). AppKit:
  system-drawn toolbar items (labels, overflow, customization,
  NSToolbarItemGroup runs), badges, edge-to-edge content via
  NSBackgroundExtensionView, ~50 new SF Symbol mappings, window `toolbarStyle`/
  `frameAutosaveName`/density, inspector split slot. **Box.spacing default is
  now the platform standard (8 AppKit / 6 GTK) instead of 0**; the schema
  default `-1` is the sentinel for it.
- **Feature detection + app state**: `hasWidget(type)` / `hasCommand(type,
  command)` (`packages/react/src/platform.ts`) answer from a helloAck host
  manifest, replacing try/catch around `sendCommand` (fallback semantics for
  hosts predating the fields). `app.isActive()` is synchronous, kept current by
  host-side state replay after HelloAck. `NotificationOptions.data` is echoed
  on the notification click event.
- **npm publishing + release**: everything ships under `@nativedesktop/*`;
  `packages/nd` publishes as **`@nativedesktop/cli`** (bin `nd`: `nd dev` /
  `nd build` / `nd package [mac|linux]` / `nd doctor`). The packaging pipeline
  lives in `packages/nd/src/package/` (identity, icons, payload, mac, linux,
  updates, doctor); `tools/package.ts` is a legacy shim that packages the
  gallery example through it. `@nativedesktop/host` resolves the prebuilt host
  binary from optionalDependencies `@nativedesktop/host-darwin-arm64` /
  `host-linux-x64`, building in place only inside a framework checkout;
  `ND_HOST_BINARY=<path>` overrides all of that, and is the only way through
  `nd dev` on a machine that cannot run a generic prebuilt (NixOS) or has none
  (gtk on macOS). `.github/workflows/release.yml` (v* tags, or manual dry-run):
  builds both hosts (macos-26 for Swift tools 6.2; ubuntu:26.04 container so
  the GTK host links by soname against libadwaita 1.7, not nix RPATHs), stages
  them into the platform packages, checks lockstep versions, publishes, and
  attaches raw binaries to a GitHub release. **Commit a version bump together
  with its `bun install`**: `bun pm pack` rewrites `workspace:*` from the lock,
  and `--frozen-lockfile` does not object to a workspace version the lock
  predates, so v0.1.1 published eleven 0.1.1 packages that each depended on
  0.1.0 siblings — consumers resolved 0.1.1 TypeScript onto 0.1.0 host
  binaries. `scripts/release/check-versions.ts` fails on that now.
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
  `style.zig`, `terminal.zig`, `tabs.zig`, `sourcetree.zig`, `audio.zig`.
- **AppKit backend:** `swift/Sources/NDShell/` (hand-written) +
  `swift/Sources/NDGen/` (generated) + `swift/Sources/CNd/` (bridges `libnd.a`);
  build scripts under `scripts/mac/`.
- **JS packages:** `packages/react` (`@nativedesktop/react`: reconciler,
  intrinsics, `Platform`, paths, store, errors), `packages/nd`
  (`@nativedesktop/cli`: the `nd` bin + packaging pipeline), `packages/host`
  (+ `host-darwin-arm64`/`host-linux-x64` prebuilt binaries), `packages/data`
  (worker SQLite), `packages/rpc`, `packages/panes`, `packages/test`
  (automation harness), `packages/mcp` (MCP bridge),
  `packages/babel-plugin-nativedesktop`.
- **Schemas:** `schema/{widgets,protocol,rpc}.json` → `tools/codegen.ts` →
  generated Zig/TS/Swift.
- **Build:** `zig build` → `zig-out/bin/nd-hello` (Zig host); the SwiftPM
  package in `swift/` builds the AppKit shell; `bun run dev` == `nd dev`.
