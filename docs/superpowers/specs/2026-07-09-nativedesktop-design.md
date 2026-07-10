# NativeDesktop — Design Specification

**Date:** 2026-07-09 (working title "NativeDesktop"; final name TBD by owner)
**Status:** Draft for owner review. No implementation beyond Milestone 1 scaffolding until approved.
**Provenance:** Synthesized from a 16-agent research/design workflow (10 web researchers on the July-2026 tech landscape, 3 independent architecture proposals, 3-lens judge panel). Full corpus: `docs/research/2026-07-09-ultracode-research.json`.

## 1. What this is

A cross-platform desktop framework where developers write **React/TSX in TypeScript** and get **real native widgets** — GTK4 on Linux, AppKit on macOS, Win32/Direct2D Fluent-drawn widgets on Windows. No webview on the UI path. The core is **Zig**; the app logic runs on **Bun**. The framework is **agent-first**: every app carries a built-in automation layer (widget-tree dump with stable IDs, in-process screenshots, semantic input synthesis, headless CI mode, first-party MCP server) so AI coding agents can build, run, see, and drive apps autonomously.

Positioning: between Electron (bundles Chromium, ~100MB+, non-native UI) and Tauri (small, but webview UI). We bundle a JS runtime but not a browser: honest footprint ~60MB uncompressed / ~15–40MB compressed. We do not promise Tauri-class sizes.

### Goals
- React 19 DX (hooks, Suspense, transitions, HMR, React Compiler) driving native widget trees.
- Native look, feel, and performance per platform; 60fps under JS load.
- Extensible at every layer: TS plugins, native (Zig/C-ABI) plugins, sandboxed WASM plugins, and swappable UI backends (a CEF/webview backend must be able to slot in later without touching the renderer or protocol).
- Agent-first automation built into the core, not bolted on. Works even when the JS side hangs.
- One widget schema, three backends; missing widgets get composited, custom-drawn, or escape-hatched — never silently dropped.

### Non-goals (v1)
- Mobile targets, SSR, web output.
- Matching Tauri binary sizes.
- Auto Layout/constraint-based layout; we use native containers with a constrained prop set.
- Supporting arbitrary app-chosen React versions (the reconciler pair is vendored; see §5).

## 2. Decision record

These decisions came out of the research + judged design panel. The judge split: Proposal 2 (pragmatic two-process) won the feasibility lens 8.5/10; Proposal 3 (versioned platform architecture, same topology) won the performance and DX/longevity lenses 8.5/10 each; Proposal 1 (in-process Bun host) lost primarily on one bet. The adopted design is Proposal 2's skeleton with Proposal 3's contracts and specific Proposal 1 details grafted on.

| # | Decision | Rationale (key evidence) |
|---|----------|--------------------------|
| D1 | **Two processes: Zig host owns `main()` and the native UI loop; Bun runs the React app as a child.** | AppKit requires the first thread and Bun exposes no run-loop pumping API; cooperative pumping of GTK/NSApp from Bun's tick is unproven at scale (the losing proposal's own "least-proven piece"). Ghostty proves the Zig-core-owns-the-loop pattern. Crash isolation: JS death leaves windows up and the automation server answering. |
| D2 | **Bun pinned at 1.3.13, integrated only via stable process-level features (spawn, sockets, bundler). The boundary is engine-agnostic.** | Bun's Zig lineage is dead: post-acquisition Rust rewrite merged May 2026; 1.3.14 is the last Zig-based release; no libbun exists (oven-sh/bun #12017 open). `Bun.spawn({ipc})` is Bun-to-Bun only; `bun:ffi` is officially experimental with JSCallback thread bugs (#15925, #7582) and a July 2026 `--compile` dlopen regression (#30717). Anything that speaks NDP over a socket is a valid runtime — the hedge against 1.4.x churn and the door to QuickJS/Node adapters. |
| D3 | **Protocol ("NDP"): length-prefixed (u32 LE) JSON frames over a unix domain socket / named pipe, exactly one `CommitBatch` per React commit, with a version handshake from day one.** | Batched-per-commit JSON is fast enough at 60fps for realistic trees (per-message encode/decode was Electrobun's pre-v1 pain, not batching); it is greppable by humans and agents (`NDP_TRACE=1`). The handshake (`hello` + protocol version + capability flags) costs a day now and makes the binary migration and non-Bun runtimes negotiable instead of a flag day — versioning cannot be retrofitted without breaking early adopters. |
| D4 | **A binary command-buffer encoding is specified early (during M3) but shipped late (M10), 1:1 with the JSON op list.** | Fixed-layout opcode stream (opcode u8, generation-tagged u32 node IDs, interned string table) documented as a schema while JSON ships; the two encodings share one op list so the fast path is a drop-in when 10k-node mounts demand it. |
| D5 | **Zig 0.16.0 exactly pinned (vendored toolchain + Nix flake); one deliberate migration per Zig release; std.Io.Threaded for aux I/O; UI main loops stay native.** | Writergate + std.Io broke virtually all pre-2025 Zig idiom; 0.17 (~Aug 2026) reworks the build system; Ghostty/TigerBeetle both pin-and-migrate. Nobody has integrated std.Io's event loop with GLib/NSRunLoop — don't be first. `@cImport` is gone: C translation goes through the build system (`translate-c` package). |
| D6 | **All widget bindings are code-generated from machine-readable metadata (GIR via zig-gobject on Linux, widget schema everywhere, win32metadata on Windows). Hand-written per-widget bindings are banned.** | The documented death of Proton Native, React-NodeGui, and Yue: bus-factor-1 maintainers crushed by "add more components" against hand-written bindings. |
| D7 | **macOS backend is a thin Swift/AppKit shell over the C-ABI Zig core (Ghostty's libghostty pattern), not pure-Zig objc.** | Mitchell Hashimoto's explicit guidance; SwiftUI escape hatch (NSHostingView) is only reachable from a Swift shell; zig-objc reserved for small in-core needs. Boundary stays plain C (Swift-C++ interop still "actively evolving"). |
| D8 | **Windows backend is raw Win32 windowing (zigwin32) + custom-drawn Direct2D/DirectWrite Fluent widgets, each with a UIA provider and AutomationId from day one.** | Stock common controls are Windows-8-era with no official dark mode; WinUI has no C projection and Zig's MinGW target can't compile C++/WinRT (ziglang/zig #23115); Microsoft's own retreat (RNW renders via Composition) validates drawing over binding XAML. Budget 2–3× the other backends; ship Windows last. |
| D9 | **Layout is delegated to native containers (GtkBox/GtkGrid, NSStackView with `isFlipped` containers and manual frames, a small Zig stack/grid for Windows). No Yoga port.** | Kills most synchronous cross-boundary layout reads by construction; a constrained Box/Grid prop set (direction, spacing, padding, align, expand) every backend can honor. |
| D10 | **Synchronous reads (text measure, getBoundingRect) get three layers: post-commit geometry push-back into a JS-side cache; a dedicated sync socket (blocking `fs.readSync`, ~10–50µs) for misses; an optional in-process N-API module (`@nativedesktop/measure`) exposing only measure/geometry reads for text-heavy apps.** | useLayoutEffect and measure callbacks are synchronous and cannot be served async; native-container layout makes them rare, the cache makes most of the rest free, and the N-API escape hatch avoids abandoning the two-process default. Sync reads are lint-flagged as slow-path. |
| D11 | **Agent-first automation lives in the Zig host, shares the renderer's retained widget tree, and must answer (tree dumps, screenshots) even while the JS thread is stalled — an explicit, tested SLO.** | See §8. |
| D12 | **Security: Tauri-v2-style capability ACL at the NDP dispatch layer; untrusted plugins in wasmtime/Extism WASM sandboxes; Bun's lifecycle-script blocking preserved in the CLI.** | JS is untrusted by default; per-window namespaced grants double as dead-code-elimination manifest; npm preinstall scripts are the dominant supply-chain vector (Shai-Hulud). |

## 3. Architecture overview

```
┌─────────────────────────────── app process tree ───────────────────────────────┐
│                                                                                │
│  Zig HOST (owns main(), native UI loop)          Bun CHILD (React app)         │
│  ┌──────────────────────────────────┐            ┌──────────────────────────┐  │
│  │ UI thread: GLib main loop /      │  NDP       │ @nativedesktop/react     │  │
│  │  NSApplication.run (Swift shell)/│◄══════════►│  (vendored react@19.2 +  │  │
│  │  Win32 message pump              │  socket    │   react-reconciler@0.33) │  │
│  │  • retained widget tree          │  1 frame = │ app code (TSX)           │  │
│  │  • applies CommitBatch on UI     │  1 commit  │ TS plugins (npm)         │  │
│  │    thread, 1 closure per commit  │            └──────────────────────────┘  │
│  │  • codegen'd widget bindings     │  sync socket (measure/geometry misses)   │
│  ├──────────────────────────────────┤  events (host→runtime, prioritized)      │
│  │ reader thread (std.Io.Threaded)  │                                          │
│  │ automation thread: JSON-RPC over │◄─── agent / MCP client / CI harness      │
│  │  local WebSocket + stdio         │                                          │
│  │ capability ACL at NDP dispatch   │                                          │
│  └──────────────────────────────────┘                                          │
│  native plugins (.so/.dylib/.dll, C ABI) · WASM plugins (wasmtime/Extism)      │
└────────────────────────────────────────────────────────────────────────────────┘
```

- **Host** holds the authoritative retained tree keyed by stable node IDs `(generation: u16, id: u32)`. Mutation batches are applied on the UI thread via `g_main_context_invoke_full` / `dispatch_async_f`(main queue) / `PostMessage` — one closure per React commit, never per mutation; on GTK inside a frame-clock tick callback (`gtk_widget_add_tick_callback`) for vsync alignment (macOS: CVDisplayLink-aligned drain; Windows: DWM-synced apply).
- **Child** is spawned with the socket path in its environment. JS crash/hang: host stays up, renders an in-window error overlay, automation keeps answering, and the host can restart the child and re-mount against generation-tagged IDs. `kill -9` of the child with surviving windows is a permanent CI regression test.
- A native-toolkit crash kills the app — the same failure mode every native app has.

## 4. NDP protocol

Framing: `u32 LE length ‖ UTF-8 JSON`. Handshake first: runtime sends `hello { ndpVersion, runtime: {name, version}, capabilities: [] }`; host replies `helloAck { ndpVersion, encodings: ["json"], widgets: <schema version>, capabilities }`. Version mismatch fails loudly with both versions in the error.

Message families:
1. **runtime→host `CommitBatch`** `{ commitId, generation, ops: [...] }` — ops: `create {id, type, props}`, `append {parent, child}`, `insertBefore {parent, child, before}`, `remove {id}`, `update {id, props}` (renderer diffs old/new itself — `prepareUpdate` is gone in React 19), `setText {id, text}`, `hide {id}` / `unhide {id}` (Suspense).
2. **host→runtime `Event`** `{ seq, priority: discrete|continuous|default, nodeId, name, payload }`. Host-side coalescing rules (adopted verbatim from the performance graft): **discrete events (click, key) are never dropped and preserve order; continuous events (scroll, motion, drag) are keep-latest per node**. Priorities map to react-reconciler lanes: discrete→`DiscreteEventPriority`, continuous→`ContinuousEventPriority` — so `startTransition` works day one.
3. **host→runtime `Geometry`** — post-commit push of computed layout `{ commitId, rects: [{id, x, y, w, h}] }` (logical units) feeding the JS-side geometry cache (D10).
4. **control** — `windowCreate`, `windowClose`, `appQuit`, `ping/pong`, `automationHello` (see §8).

Sync channel: second socket; runtime writes a request (`measureText`, `getRect` cache miss) and blocks on `fs.readSync(fd)`; the host's UI thread answers. Windows named-pipe equivalence for the blocking read must be validated in M7 (known under-specified spot).

`NDP_TRACE=1` pretty-prints every frame on both sides; the tracer is part of the protocol layer, and when the binary encoding lands the same tracer decodes it to identical JSON text (greppability survives the binary migration).

## 5. React renderer — `@nativedesktop/react`

- react-reconciler **0.33.0 in mutation mode**, **bundled together with react@19.2.x** in one package (react-three-fiber v9.5 pattern). Reason: the host config breaks on React *minors* (19.1→19.2 broke every unbundled renderer: "resolveUpdatePriority is not a function"). App React versions never float against the reconciler; every React upgrade is a scheduled migration.
- Render-phase host config (`createInstance`, `appendInitialChild`, `finalizeInitialChildren`) is side-effect-free: cheap JS descriptors with monotonically assigned generation-tagged node IDs. Concurrent React renders and discards work-in-progress trees; real widgets are created **only at commit** (else native objects leak).
- Commit phase appends ops to the current batch; `resetAfterCommit` flushes exactly one `CommitBatch`. `getRootHostContext`/`getChildHostContext` return non-null sentinels (React 19 treats null as "missing"). `hideInstance`/`unhideInstance` → `gtk_widget_set_visible` / `NSView.isHidden` / `ShowWindow`.
- Hot reload: Bun HMR + react-refresh wired through the reconciler's `setRefreshHandler`, reconciler and refresh runtime in the same bundle chunk (registration must precede any React code). Full reload bumps the generation counter; the host garbage-collects orphaned-generation widgets.
- React Compiler 1.0 ships enabled in the app template — renderer-agnostic, directly cuts commit traffic.
- Text measurement is a host-side service (Pango / NSAttributedString / DirectWrite) reached via the D10 ladder.

## 6. Widget system

**One machine-readable widget schema** (`widgets.schema.json`, versioned) is the contract; backends fill it in. Codegen emits: TS/JSX intrinsic types, Zig prop-appliers, docs, and the automation layer's role/property tables. v1 core set (~20): Window, Box, Grid, Label, Button, TextInput, TextArea, Checkbox, Radio, Select, Slider, ProgressBar, Image, ScrollView, ListView, TabView, Menu, Separator, Spinner, WebView-stub.

- **Linux:** plain GTK4 ≥4.20 (tested 4.22) via zig-gobject master (Zig 0.16 support landed April 2026 — pin a post-April commit and vendor it; the binding generator needs `xsltproc`, provided by the flake). libadwaita is an optional all-or-nothing add-on package, never core (adw_init takes over theming). Lists: `GtkListView` + `GtkSignalListItemFactory` with a React item-template component driving setup/bind/unbind — 100k-row recycling is an M5 demo fixture.
- **macOS:** Swift/AppKit shell (SwiftPM) linking the Zig core static lib over a flat C command/callback surface. NSApplication on the main thread; mutations via `dispatch_async_f` (never `dispatch_sync` from main — instant deadlock). Manual `setFrame` layout with `isFlipped` overridden on **every** container class (not inherited). `NSHostingView` keeps SwiftUI reachable as an escape hatch.
- **Windows:** zigwin32 windowing, per-monitor-v2 DPI from day one, dark title bars via `DWMWA_USE_IMMERSIVE_DARK_MODE` plus the version-gated uxtheme ordinal technique (ordinals 133/135 are undocumented and have changed semantics — OS-build-gated, never load-bearing). Widgets custom-drawn Fluent via Direct2D/DirectWrite; **every widget implements a UIA provider with AutomationId** — that single investment buys the automation layer, OS-level agent interop (Microsoft's agentic-Windows push rides on UIA), and screen readers simultaneously.

**Styling** is an explicit non-web `style` prop (colors, fonts, padding, margins, borders) compiled to GtkCssProvider rules / AppKit appearance properties / Direct2D brushes, **validated at commit against the schema**: hallucinated web CSS (`flex`, `position`, `display`) fails loudly with a fix-it message. This is an agent-facing feature — LLMs reliably hallucinate web CSS at GTK.

**Missing-widget ladder** (in order): (1) composite from schema primitives in TS (`@nativedesktop/widgets`); (2) custom-drawn in the Zig core with an AccessKit-backed accessibility node (AccessKit's C bindings; the same crate GTK 4.18+ itself uses on Windows/macOS); (3) platform escape hatch — a typed extension op letting a native plugin own a subtree (SwiftUI island, GTK custom widget, `DesktopWindowXamlSource` island).

**Null backend:** a headless in-memory backend implementing the same backend interface, used by the conformance suite to pin schema semantics before (and independently of) each platform backend — and the standing proof that a CEF/webview backend can slot in later.

## 7. Backend interface

Backends implement a C vtable — create/destroy/reparent widget, set property, run-loop hooks, snapshot, dispatch-semantic-action, measure — negotiated by `abi_version`. GTK4 is the reference implementation; the conformance suite runs against null + GTK from M5, macOS from M6, Windows from M7. A CEF backend later maps NDP widget ops to browser-shell surfaces and brings its own process tree under the host, touching neither renderer nor protocol.

## 8. Agent-first automation layer

Lives in the Zig host, enabled by `NATIVE_AUTOMATION=1` (always-on in dev; opt-in flag for production builds). Shares the renderer's retained widget tree — **SLO: tree dumps and screenshots answer from the mutex-protected retained tree even while the JS thread is stalled** (tested by stalling JS in CI and asserting the automation server still responds).

Surface: JSON-RPC over a local WebSocket + stdio, with a first-party **MCP wrapper package** so Claude/other agents get tools directly. Two tiers:

1. **Semantic (default, Playwright-MCP model):** `getTree`/`snapshot` — structured text snapshot with stable refs (node ID + developer `testID` prop), roles, names, logical-unit geometry with an **explicit coordinate-space contract**; `click/type/setValue/scroll/focus(ref)` as semantic action dispatch on the UI thread (GTK4 removed synthetic-event injection — GdkEvent structs are private — so semantic dispatch is the only correct Linux model, matching UIA InvokePattern / AXPress); **actionability hit-tests before dispatch** (test the ref's center against the live tree; fail loudly if covered/invisible/off-screen — semantic dispatch must not click what a user couldn't); `waitFor(condition)`.
2. **Raw (computer-use compatibility):** `screenshot(window|node)` + coordinate `click/key/scroll` with explicit HiDPI logical↔pixel remapping, so Anthropic computer-use loops work without the semantic layer.

Screenshots are **in-process renders, never OS capture**: GTK4 `gtk_widget_snapshot` → GskRenderNode → `gsk_renderer_render_texture` → PNG; macOS `NSView.cacheDisplay` (no TCC prompt for your own views — ScreenCaptureKit prompts, nags monthly, and fails silently when unauthorized); Windows `PrintWindow(PW_RENDERFULLCONTENT)` with `Windows.Graphics.Capture` fallback (GPU-composited content can come back black). Known limitation to document: in-process snapshots miss OS decorations, popovers/sheets on separate surfaces, and externally composited content — the API enumerates surfaces and composites what it owns.

Optional **"real OS input" mode** for end-to-end realism: `SendInput` (Windows, permission-free), `CGEventPost` (macOS, needs Accessibility TCC), libei/portal on Linux where available — never the default (Wayland blocks global injection; portals require interactive consent).

Accessibility mirroring: native widgets give AT-SPI2/NSAccessibility/UIA trees nearly free; custom-drawn widgets go through AccessKit. Our own tree-dump protocol is primary (AT-SPI needs an a11y bus that bare CI containers lack).

**Headless CI:** Linux — `weston --backend=headless` + `GSK_RENDERER=cairo` (explicitly not Broadway or X11: both deprecated for removal in GTK 5); proven in **Milestone 1**, before anything is built on top. macOS/Windows — no true headless exists; stock GitHub Actions runners work headful because the in-process design needs zero TCC/permissions.

**v1 transport deviation (M4):** M4 ships the JSON-RPC surface above over a second **framed unix socket** (the same u32-length-prefixed JSON framing as the NDP protocol), not the WebSocket + stdio named above. WebSocket support is deferred to a later milestone. The first-party MCP wrapper (`packages/mcp`) still provides the stdio+MCP half of the surface for agents — it speaks stdio MCP outward and bridges to the framed unix socket inward — so this is a wire-level substitution, not a scope cut.

## 9. Extensibility & security

Three plugin tiers crossing the same audited boundaries:
1. **TS plugins:** plain npm/Bun packages in the Bun process. CLI preserves Bun's lifecycle-script blocking (`trustedDependencies`) and enforces lockfile + `bun audit`.
2. **Native plugins (first-party/audited):** Zig or any C-ABI shared library implementing a semver'd `nd_plugin_v1` struct (`abi_version`, capability declarations, init/deinit), registering widget types into the schema registry and new NDP commands; a metadata manifest feeds codegen so TS types appear automatically. Deep-signed with JIT entitlements on macOS.
3. **Untrusted plugins:** WASM components under wasmtime via Extism (Zed's extension system as the model), deliberately minimal host-function surface (2026 wasmtime CVEs show the host API is the real sandbox boundary).

Every command JS can invoke passes a **Tauri-v2-style capability ACL** enforced at NDP dispatch in the Zig core: per-window grants, namespaced `plugin:permission`, ACL doubles as a dead-code-elimination list. JS is untrusted by default.

## 10. Developer experience

- `bun create nativedesktop` template: TSX counter app, React Compiler on, `NDP_TRACE` documented.
- `nd dev` (build + run + HMR), `nd package`, `nd doctor` (toolchain pin verification).
- In-window error overlay rendered **by the host** when the runtime crashes or disconnects.
- **Agent docs are a framework deliverable** (M8): current-idiom Zig 0.16 and GTK4 snippets (LLMs trained on pre-2025 Zig hallucinate removed APIs like `std.io.getStdOut().writer()`), the style-lint rules, and MCP tool documentation. Shipped as `AGENTS.md`/`CLAUDE.md` templates in the app scaffold.

## 11. Packaging, updates

`nd package` per platform: macOS .app with hardened runtime + `com.apple.security.cs.allow-jit` (JSC under Bun crashes on Apple Silicon without it), deep-signing of nested binaries, notarytool + staple (Sequoia removed the Gatekeeper bypass; unnotarized apps are effectively dead). Windows: signed NSIS installer + winget manifest; Azure Trusted/Artifact Signing documented as default cert path (geo-restricted; no instant SmartScreen trust — set expectations). Linux: Flatpak manifest (GNOME 50 runtime; automation documented as in-process-first since portals gate everything else) + AppImage nightlies. Updates in the Zig core: minisign/Ed25519-verified manifests (non-disableable), zig-bsdiff+zstd deltas chaining only from the previous version, full-download fallback always hosted.

## 12. Testing strategy

- **Protocol:** golden-frame tests for NDP encode/decode on both sides; handshake version-mismatch tests.
- **Renderer:** host-config unit tests against a mock transport (descriptor purity: render-discard leaks nothing); Suspense hide/unhide; priority-lane mapping.
- **Backends:** conformance suite from the widget schema against null backend first, then each platform; per-widget prop-applier tests.
- **Crash isolation:** `kill -9` Bun child → window survives + error overlay + automation answers (CI, every commit).
- **Automation SLO:** stall JS thread → `getTree`/`screenshot` still answer.
- **Performance gates:** 10k-node mount benchmark (JSON soft spot; also gates the binary path when it lands); 60fps scroll-under-JS-load trace.
- **Twin-binding conformance (when `@nativedesktop/measure` lands):** the same C header bound via N-API and (experimental) bun:ffi runs one shared suite per commit so the fast lane can't silently diverge.

## 13. Risks (accepted, with mitigations)

1. react-reconciler breaks on React minors, no semver → vendored pair; upgrades are scheduled migrations.
2. Bun 1.3.x is an unmaintained lineage if 1.4 (Rust) stabilizes slowly → engine-agnostic NDP in practice, validated by a QuickJS-or-Node adapter spike before v1.0.
3. Windows is 2–3× the effort of other backends → ship last (M7); custom-draw + UIA from day one, never retrofit.
4. Widget-breadth demand (killed all predecessors) → codegen mandate (D6) + missing-widget ladder + schema-driven conformance.
5. Sync measurement over IPC (~10–50µs/call) hurts text-measure-heavy apps → D10 ladder; lint flags sync reads; N-API escape hatch.
6. Zig 0.17 (~Aug 2026) build-system churn → thin, centralized build.zig; pinned toolchain; scheduled migration.
7. Flatpak sandbox constrains automation → in-process-first automation everywhere; portals only for the opt-in OS-input mode.
8. Large initial mounts on JSON → binary command buffer specified early (D4), drop-in later; benchmark is a permanent gate.

## 14. Milestone roadmap

Each milestone is independently demoable; detailed bite-sized plans are written per-milestone (M1's exists: `docs/superpowers/plans/2026-07-09-m1-window-from-zig.md`).

- **M1 — Window from Zig (Linux) + headless CI proof.** Pinned toolchains (Zig 0.16.0, Bun 1.3.13, Nix devshell + xsltproc), content-hash-pinned post-April-2026 zig-gobject (physical vendoring deferred until upstream risk materializes); `zig build run` opens a GTK4 window with a clickable button; the same app runs under `weston --backend=headless` + `GSK_RENDERER=cairo` in CI. *(Headless proof pulled forward from M4 by judge graft.)*
- **M2 — NDP + Bun child.** Handshake with version negotiation; plain TS script (no React) builds a Box/Label/Button tree via CommitBatch and receives Events; `NDP_TRACE=1`; kill -9 crash-isolation demo becomes a CI test.
- **M3 — React renderer.** `@nativedesktop/react` (react@19.2.x + react-reconciler@0.33.0, mutation mode); counter app with useState, Suspense, startTransition against live GTK. Binary command-buffer spec document written now (D4).
- **M4 — Automation layer v1.** JSON-RPC server + MCP wrapper; getTree/screenshot/semantic input/waitFor; agent-drives-the-counter-app-headless demo; automation SLO test.
- **M5 — Widget schema + codegen + styling.** ~20 widgets on GTK; GtkListView 100k-row recycling; style validation rejecting web CSS; null-backend conformance suite.
- **M6 — macOS backend.** Swift shell + Zig static lib (Ghostty pattern); TCC-free automation (cacheDisplay, postEvent, accessibilityIdentifier); same apps + MCP drive in CI. Dev loop uses a real Apple-silicon Mac over `ssh macbook` (logged-in, nix devshell): build, run, and in-process screenshot the backend directly over SSH — `cacheDisplay` needs no TCC/GUI-capture grant, so an SSH session captures fine without a headful capture session.
- **M7 — Windows backend.** zigwin32 + D2D Fluent widgets with UIA providers/AutomationId; PrintWindow screenshots; validate sync-channel named-pipe semantics; inspectable by Accessibility Insights.
- **M8 — DX.** HMR + react-refresh with generation GC; host error overlay; `bun create nativedesktop`; agent docs (current-idiom Zig/GTK snippets, lint rules).
- **M9 — Packaging + updates.** Signed/notarized/auto-updating gallery app on all three platforms from one CI workflow.
- **M10 — Plugins + hardening + binary fast path.** Capability ACL enforcement; `nd_plugin_v1` ABI; wasmtime/Extism tier; binary command buffer behind the 10k-node benchmark.

## 15. Open questions (owner input wanted, none blocking M1)

1. **Name.** "NativeDesktop" is the directory name; a real name/npm scope is needed before M3 publishes packages.
2. **License.** MIT/Apache-2.0 dual is the ecosystem default; affects AccessKit (MIT/Apache) and Extism integration trivially.
3. **libadwaita add-on priority** — ship the optional package at M5 or defer post-v1?
