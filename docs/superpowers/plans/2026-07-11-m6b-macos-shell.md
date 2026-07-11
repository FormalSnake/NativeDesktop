# M6b — Swift/AppKit shell + widget parity + TCC-free automation: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Parent plan.** This is the full expansion of the M6b outline at the end of `docs/superpowers/plans/2026-07-10-m6-macos.md`. **M6a is landed and green** (`84b58e9`): the core (`tree`/`runtime`/`automation`/`protocol`) is GTK-free behind the C vtable in `include/nd.h`; `zig build libnd -Dbackend=abi` produces `zig-out/lib/libnd.a` + `zig-out/include/nd.h`, and it builds natively on the Mac over ssh (`MAC_LIBND_OK`). M6b is the **Swift/AppKit embedder** that links that static lib — the AppKit analog of `src/gtk/main.zig` + `src/gtk/backend.zig`, one-for-one with every vtable field the GTK embedder fills.
>
> **What lands.** A SwiftPM package at `swift/` that links `libnd.a`, an `NSApplication` main that fills an `nd_backend` from a codegen-emitted `Widgets.swift`, the 18-widget AppKit backend, `NSView` in-process TCC-free screenshots with a fidelity fallback ladder, semantic input, and the counter + gallery demos driven by the existing MCP `AutomationClient` over `ssh macbook`. No changes to the C ABI (`include/nd.h` is frozen — M6a-D1), no schema changes, no Linux behaviour changes.

---

## Goal (M6b)

Stand up the **macOS embedder** on top of the frozen `libnd` C ABI so that the *same* core (runtime/tree/automation/protocol that Linux runs) drives native AppKit widgets:

1. **SwiftPM shell links `libnd.a`.** A `swift/Package.swift` with a `CNd` system-library target (a hand-written `module.modulemap` exposing `include/nd.h`) and an executable target that links `zig-out/lib/libnd.a`. An `NSApplication` main fills an `nd_backend` vtable, calls `nd_init`/`nd_register_backend`/`nd_start_runtime`/(conditionally)`nd_start_automation`, and runs `NSApp.run()` on the main thread — the AppKit peer of `src/gtk/main.zig`.
2. **Codegen emits the Swift widget layer (D6).** `tools/codegen.ts` gains a `genSwift(schema)` emitter that mirrors `genZig`'s per-widget dispatch structure (create dispatcher, applyProps, connectEvents, structural ops), throwing on any untemplated widget exactly like the Zig side (`no create template for widget ${w.name}`), and writes `swift/Sources/NDGen/Widgets.swift`. **The existing generated files' bytes are unchanged** — Swift is a new output path.
3. **The 18 widgets render + automate natively.** Every `schema/widgets.json` widget gets an AppKit mapping; the backend fills all 18 `nd_backend` fields with AppKit code that mirrors `src/gtk/backend.zig`'s shape: structural ops, `marshal_async` via `dispatch_async_f`, `show_overlay`, `node_visible`/`node_bounds`, `snapshot` (with the fidelity ladder), and `semantic_action` dispatching click/setValue/type/scroll.
4. **Automation is TCC-free and answers under child SIGSTOP.** `getTree`/`screenshot`(non-blank)/`click`/`setValue`/`type`/`scroll`/`waitFor` answer over the Mac automation Unix socket, driven headful-in-session by `scripts/mac/mac-m6.sh` running `bun scripts/m6-drive.ts` **on the Mac** (mirroring how `mac-build.sh` runs `zig build` on the Mac). The D11 SIGSTOP-SLO is a **core** property (proven on Linux in M6a) and re-verified on Mac here.

**Non-goals (M6b):** cross-compilation (Zig builds `libnd.a` natively on the Mac); a real WebView (stays a stub, exactly like the GTK path); any schema/React/TS-renderer change; any `include/nd.h` change (the ABI is frozen).

---

## Global Constraints

Carried from M6a + the Mac probe facts (all probed 2026-07-10 from **this Linux box over plain `ssh macbook`**, non-interactive — reproduced here, do NOT re-probe):

- **Host:** `ssh macbook`, arm64, macOS 26.x ("Tahoe"), Xcode 26.4.1 (build 17E202), Swift 6.3.1 (swiftlang-6.3.1.1.2), SwiftPM usable (`swift package --version`), target `arm64-apple-macosx26.0`. macOS SDK at `/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk`.
- **GUI session is uid `502`** (not 501). Every headful script uses `$(id -u)`, never a hard-coded uid. `launchctl print gui/$(id -u)` → `GUI_OK`.
- **Toolchain is the profile PATH, NOT nix.** `zig 0.16.0` + `bun 1.3.13` live at `/etc/profiles/per-user/kyandesutter/bin`. **nix is NOT usable non-interactively over ssh** — never `nix develop` on the Mac. All Mac scripts `export PATH="/etc/profiles/per-user/kyandesutter/bin:$PATH"` and invoke `zig`/`bun` directly; `swiftc`/`swift`/`xcrun` come from Xcode.
- **`NSApplication.run()` on the main thread.** The C core calls *up* through the vtable on the embedder's UI thread; the core marshals to it via `vtable.marshal_async`. On Mac that is **`dispatch_async_f(dispatch_get_main_queue(), ctx, fn)`** — the libdispatch C API, taking a C function pointer, NOT a Swift block. **NEVER `dispatch_sync` from the main queue (deadlock).**
- **`isFlipped == true` on EVERY container view class**, not inherited — a bare `NSView` is bottom-left-origin. The self-review checks that every container class M6b defines overrides `isFlipped`. This makes the layout model top-left y-down, matching `getTree`'s `logical-window-topleft` coordinate contract.
- **Screenshots are TCC-free in-process only.** `CGWindowListCreateImage` and any screen-capture API that triggers a TCC screen-recording grant are **FORBIDDEN** — the whole point of the probe was proving `NSView` in-process capture needs no grant. The probe proved the *pipeline* is TCC-free but **naive `cacheDisplay` returns blank subviews** (see T5's fidelity ladder — the one carried-forward risk).
- **`NSHostingView` (SwiftUI escape hatch) is documented but NOT used** in M6b.
- **The whole Linux gate must stay green throughout.** M6b touches `tools/codegen.ts` (adds `genSwift`) — the existing generated bytes must not shift. Run the M6a gate before starting and after any codegen change:

```bash
nix develop -c bash -c 'bun tools/codegen.ts \
  && git diff --exit-code -- schema packages/react/src/generated src/generated docs/widgets.md docs/styling.md \
  && zig build test \
  && zig build \
  && bun install --frozen-lockfile \
  && ./scripts/headless-smoke.sh \
  && ./scripts/headless-m2.sh \
  && ./scripts/kill9-test.sh \
  && ./scripts/headless-m3.sh \
  && ./scripts/headless-m4.sh \
  && ./scripts/headless-m5b.sh \
  && ./scripts/headless-m5c.sh \
  && ./scripts/headless-m8.sh'
```

- **SwiftPM package lives at top-level `swift/`** (the M6b outline names `swift/Package.swift` in T1 — use `swift/`, not `macos/`). Sources under `swift/Sources/`, the module map under `swift/Sources/CNd/`.
- **Commit style:** short imperative lowercase subject, conventional prefix (`feat(swift):`, `feat(codegen):`, `feat(appkit):`, `feat(mac):`) matching recent history (`84b58e9 fix(abi): …`, `eb4a879 build(abi): …`). No co-author trailers; `git add` explicit paths per task — never `git add -A`; `node_modules`/caches/`CLAUDE*.md` never staged.
- **The Mac dev loop is copy-then-run-remotely, NOT a socket tunnel.** The automation socket is a Unix domain socket in `$XDG_RUNTIME_DIR`-equivalent on the Mac (M4 fact: `$TMPDIR/nd-automation-<pid>.sock` on macOS since macOS has no `$XDG_RUNTIME_DIR`; the host prints `ND_AUTOMATION_LISTENING path=<abs>`). Driving it from THIS Linux box means: `scripts/mac/mac-sync.sh` rsyncs the tree, then `ssh macbook 'cd ~/nd && … bun scripts/m6-drive.ts …'` runs the driver **on the Mac against the local socket path** — exactly mirroring how `mac-build.sh` runs `zig build` on the Mac. **No `ssh -L` port-forward** (a Unix socket can't be forwarded that way, and a TCP bridge would perturb the framing the SLO test depends on).

### AppKit symbols (recorded from the M6a probes; all accepted by swiftc 6.3.1 this session)

| Need | Symbol | Probe status |
|---|---|---|
| App + loop | `NSApplication.shared`, `.setActivationPolicy(.regular)`, `.run()`, `.terminate(_:)` | ✅ ran over ssh |
| Window | `NSWindow(contentRect:styleMask:backing:defer:)`, `.title`, `.center()`, `.makeKeyAndOrderFront(_:)`, `.contentView` | ✅ |
| Flipped container | `NSView` subclass `override var isFlipped: Bool { true }` | ✅ |
| Manual layout | `NSView.frame = NSRect(...)`, `.addSubview(_:)` | ✅ |
| Controls | `NSButton(title:target:action:)`, `NSTextField(labelWithString:)` | ✅ compiled+ran |
| Main-thread marshal from C | `dispatch_async_f(dispatch_get_main_queue(), ctx, fn)` | ✅ mechanism (never `dispatch_sync` from main — deadlock) |
| Screenshot (TCC-free) | `NSView.bitmapImageRepForCachingDisplay(in:)` + `.cacheDisplay(in:to:)` + `NSBitmapImageRep.representation(using:.png,…)` | ✅ writes valid PNG TCC-free; ⚠️ subview-fidelity blank (T5 ladder) |
| testIDs | `NSWindow/NSView.setAccessibilityIdentifier(_:)` | ✅ accepted |
| GitHub remote | `origin git@github.com:FormalSnake/NativeDesktop.git` | exists → T7 stretch viable |

### The frozen C ABI M6b implements (from `include/nd.h`, M6a-landed — do NOT edit)

`nd_backend` fields, in header order (the Swift vtable fills all 18): `create`, `apply_props`, `append_child`, `insert_before`, `remove_child`, `set_text`, `set_visible`, `apply_style`, `connect_events`, `has_parent`, `unparent`, `get_window`, `marshal_async`, `show_overlay`, `node_visible`, `node_bounds`, `snapshot`, `semantic_action`. Lifecycle: `nd_init()`, `nd_register_backend(ctx, *vt)`, `nd_start_runtime(ctx)`, `nd_start_automation(ctx)`, `nd_free(p)`, `nd_emit_event(ctx, node_id, name, payload_json)`. `props_json`/`arg_json`/`style_json`/`attached_json` cross as NUL-terminated UTF-8 JSON (M6a-D2); widgets cross as `void*` (`nd_widget`); `nd_rect { int32 x,y,w,h }`. `semantic_action` returns `0` ok / negative JSON-RPC code, and mallocs `result_json_out`/`err_json_out` (freed by the core via libc `free`, uniform across languages).

---

## M6b decision record (owner-facing judgment calls, locked by this plan)

- **M6b-D1 — Props/style/args decode with `JSONSerialization`, not a codegen'd typed decoder (v1).** The ABI hands Swift a NUL-terminated JSON string per op (M6a-D2). Swift re-parses it with `Foundation.JSONSerialization.jsonObject(with:)` → `[String: Any]`, and the generated appliers read typed keys with small helpers (`propStr`/`propInt`/`propBool`/`propDouble`/`propArray`, the Swift peers of `codegen.ts`'s `ZIG_HELPERS`). **Chosen over a codegen'd `Codable` struct per widget** because it is the smallest diff, it exactly mirrors the Zig side (which re-parses with `std.json` — the GTK embedder's `parseJson`), and it needs no ABI/codegen change when a prop is added (the D6 win). A typed decoder is a later optimization, not v1.
- **M6b-D2 — ListView → `NSTableView`, single-column, view-based row recycling, `itemCount`-not-rows in `getTree`.** `<listview>` maps to an `NSScrollView` wrapping an `NSTableView` with one column and an `NSTableViewDataSource`/`Delegate` over the `items: [String]` array; rows are view-based (`makeView(withIdentifier:)` recycling), and the tracked node's `getTree` reports `itemCount` (the array length) with **zero child nodes** — recycled row views are untracked backend internals, never dumped. This is the exact AppKit analog of the GTK `GtkListView` + `SignalListItemFactory` recycling and matches M5c-D4 (`itemCount`, not 100k children). `node_bounds`/`node_visible` answer for the `NSScrollView` wrapper (the tracked widget), same as GTK's `ScrolledWindow`.
- **M6b-D3 — Box layout: `NSStackView` when the axis maps cleanly, `FlippedView` + manual `setFrame` otherwise. The specific rule:** a `<box>` whose only layout inputs are `orientation` (`vertical`/`horizontal`) and `spacing` — i.e. every current schema Box — maps to an **`NSStackView`** (`.orientation = .vertical|.horizontal`, `.spacing = spacing`, `.alignment = .leading`, `distribution = .gravityAreas`), because AppKit's stack view honours exactly those two inputs natively and re-lays out on child insert/remove without manual frame math. **`FlippedView` + manual `setFrame` is reserved for the two structural roots that need explicit flipped-coordinate control: the window `contentView` and the `ScrollView.documentView`** (both must be flipped so top-left origin holds, and both size their single child to their own bounds). No widget in the current schema needs per-pixel manual Box frames, so no `<box>` uses the manual path in v1. **`NSStackView` is itself already flipped-agnostic** (it positions arranged subviews by constraints, not raw frames), so a vertical stack lays top-to-bottom as expected under a flipped ancestor; the `isFlipped=true` constraint applies to the `FlippedView` roots and any bare-`NSView` container the backend introduces.
- **M6b-D4 — Screenshot fidelity ladder (ordered, each rung with a PASS/FAIL gate).** The probe proved naive `cacheDisplay` on the `contentView` returns a valid PNG but with **blank control subviews**. `snapshot` tries these rungs **in order**, stopping at the first that yields a non-blank PNG, where **non-blank ≡ the PNG decodes to a bitmap with >1 distinct pixel colour** (this exact criterion is the acceptance gate for `snapshot`):
  1. `window.contentView?.displayIfNeeded()` (force pending display), then `bitmapImageRepForCachingDisplay(in:)` + `cacheDisplay(in:to:)` on the `contentView`.
  2. Recursively set `wantsLayer = true` on the whole view tree, then render the content view's backing `CALayer` into a `CGContext` bitmap via `layer.render(in: cgContext)`, and wrap that as an `NSBitmapImageRep`.
  3. Per-view `lockFocus()` + `NSBitmapImageRep(focusingViewRect:)` composited manually over the tree (draw each mapped subview into one target rep at its window-space frame).
  4. `NSView.dataWithPDF(inside:)` on the content view, rasterized to a bitmap via `NSImage(data:)` → `NSBitmapImageRep`.

  Each rung writes the candidate PNG, then the **non-blank check** runs; a blank result advances to the next rung. If all four are blank, `snapshot` returns `false` and the automation server answers the RPC with the same `-32603` empty-snapshot error GTK uses. Wrapping the whole thing is a **non-blank poll loop** mirroring GTK's M5b 150ms→3s pattern: the *driver* (`m6-drive.ts`) retries `screenshot` up to 20× at 150ms spacing (exactly `m5c-drive.ts`'s loop), so a frame that isn't ready yet becomes a retry, not a failure. The chosen rung is recorded in the `snapshot` result JSON (`{"path":…,"width":…,"height":…,"rung":N}`) so the drive log shows which path rendered.
- **M6b-D5 — `semantic_action` maps to native AppKit input on the main thread (TCC-free), mirroring `src/gtk/backend.zig`'s dispatcher one-for-one.** `click` → `NSControl.performClick(nil)` (the target/action fires exactly as a real click; no `postEvent` synthesis needed for the counter/gallery widgets — `postEvent` stays available for hit-testing edge cases but is not the primary path). `setValue` → per-control (`NSTextField.stringValue` / `NSTextView.string` / `NSButton.state` for checkbox/radio / `NSSlider.doubleValue` / `NSPopUpButton.selectItem(at:)`). `type` → append to `NSTextField.stringValue` then fire the action. `scroll` → `NSScrollView.contentView.scroll(to:)` + `reflectScrolledClipView`. All run on the main thread (the automation server marshals via `vtable.marshal_async` → `dispatch_async_f`, same as GTK marshals via `invokeFull`), so the D11 SLO holds: the automation thread never touches the Bun child, only the fast main-thread backend op.

---

## TASK 1 — SwiftPM shell skeleton + link `libnd.a`

**Spine. Depends on: M6a (landed). Files: `swift/Package.swift` (NEW), `swift/Sources/CNd/module.modulemap` (NEW), `swift/Sources/CNd/shim.h` (NEW), `swift/Sources/NDShell/main.swift` (NEW), `scripts/mac/mac-run.sh` (NEW).**

Prove `libnd.a` links into a Swift `NSApplication` and the NDP handshake works cross-language: a window opens over ssh (the M6a probe, now driven by the *real* core), the counter's Bun child connects, and the first commit presents. The backend is a **stub** here (`create` returns a bare `NSView`); real widgets are T3.

- [ ] Create `swift/Sources/CNd/module.modulemap` — a system-library module exposing the frozen header (SwiftPM's canonical prebuilt-C-static-lib pattern: a `CTarget` whose module map points at a shim header that `#include`s the real `nd.h`, and a linker flag that pulls in the archive):

```
module CNd {
    header "shim.h"
    export *
}
```

- [ ] Create `swift/Sources/CNd/shim.h` — includes the installed header by absolute-relative path from the built lib. The header ships at `zig-out/include/nd.h`; the module target references it via a relative include and the package's `unsafeFlags` add the include search path:

```c
#include "nd.h"
```

- [ ] Create `swift/Package.swift`. A `CNd` system target for the header + an `NDShell` executable linking `libnd.a`. Link flags point at the Zig build output (`../zig-out/lib`, `../zig-out/include`) — the package sits at `swift/`, the artifacts at repo-root `zig-out/`:

```swift
// swift-tools-version:6.0
import PackageDescription

let repoRoot = "../"  // swift/ is one level under the repo root; zig-out/ lives at root.

let package = Package(
    name: "NDShell",
    platforms: [.macOS(.v26)],
    targets: [
        // Header-only C target: exposes include/nd.h to Swift via CNd.
        .target(
            name: "CNd",
            cSettings: [
                .unsafeFlags(["-I", "\(repoRoot)zig-out/include"])
            ]
        ),
        .executableTarget(
            name: "NDShell",
            dependencies: ["CNd"],
            linkerSettings: [
                // Link the prebuilt static lib + the frameworks the AppKit
                // backend (T3+) needs. libnd.a is GTK-free pure-Zig core.
                .unsafeFlags([
                    "-L", "\(repoRoot)zig-out/lib",
                    "-lnd",
                ]),
                .linkedFramework("AppKit"),
                .linkedFramework("Foundation"),
                .linkedFramework("QuartzCore"),  // CALayer for the T5 fidelity ladder
            ]
        ),
    ]
)
```

- [ ] Create `swift/Sources/NDShell/main.swift` — the `NSApplication` peer of `src/gtk/main.zig`. Fills a **stub** vtable, calls the lifecycle in the same order the GTK `main` does (`nd_init` → `setApp`-equivalent state → `nd_register_backend` → `nd_start_runtime` → conditional `nd_start_automation`), runs the loop on the main thread. The vtable and any context the backend needs are stored at **module/global scope** (they must outlive `main`'s frame — the core keeps the pointer for the whole process, exactly the `the_vtable` lesson from `src/gtk/main.zig:18`):

```swift
import AppKit
import CNd

// Must outlive main(): the core stores &vtable and calls through it for the
// process's whole life (mirrors src/gtk/main.zig's module-level `the_vtable`).
var gVTable = nd_backend()
var gCtx: OpaquePointer? = nil

@MainActor
func buildStubVTable() -> nd_backend {
    var vt = nd_backend()
    // T3 fills the rest from NDGen.Widgets; T1 stubs create to a bare NSView
    // so the handshake + first commit can present *something*.
    vt.create = { _, _, _ in
        let v = NSView(frame: NSRect(x: 0, y: 0, width: 100, height: 24))
        return Unmanaged.passRetained(v).toOpaque()  // handle rides as void*
    }
    vt.marshal_async = { _, fn, data in
        // dispatch_async_f: C fn ptr onto the main queue. NEVER dispatch_sync.
        DispatchQueue.main.async { fn?(data) }
    }
    vt.get_window = { _ in gWindow.map { Unmanaged.passUnretained($0).toOpaque() } }
    // (all other fields are the zero-init no-op fn ptrs at T1; T3 replaces the struct wholesale)
    return vt
}

let app = NSApplication.shared
app.setActivationPolicy(.regular)

guard let ctx = nd_init() else {
    FileHandle.standardError.write("ND_RUNTIME_ERROR nd_init failed\n".data(using: .utf8)!)
    exit(1)
}
gCtx = ctx
gVTable = buildStubVTable()
nd_register_backend(ctx, &gVTable)

if nd_start_runtime(ctx) != 0 {
    FileHandle.standardError.write("ND_RUNTIME_ERROR nd_start_runtime failed\n".data(using: .utf8)!)
    exit(1)
}
if ProcessInfo.processInfo.environment["NATIVE_AUTOMATION"] == "1" {
    if nd_start_automation(ctx) != 0 {
        FileHandle.standardError.write("ND_AUTOMATION_ERROR nd_start_automation failed\n".data(using: .utf8)!)
    }
}
app.run()
```

(`gWindow` is the module-level `NSWindow?` the stub `create` sets when it sees the `Window` kind — introduced fully in T3; for T1 the stub can open a fixed window on first `create` so the probe's `ND_PROBE_WINDOW_SHOWN`-equivalent holds.)

- [ ] Create `scripts/mac/mac-run.sh` — the headful dev-loop primitive built on `mac-sync.sh` (sync → build `libnd.a` → build Swift → run in the GUI session over ssh). Mirrors `mac-build.sh`'s `set -euo pipefail` + profile PATH + `2>&1 | tail`:

```bash
#!/usr/bin/env bash
set -euo pipefail
# Sync the tree, build libnd.a + the Swift shell, run headful in the logged-in
# GUI session. $1 = example script (default counter). Runs entirely on the Mac.
SCRIPT="${1:-examples/counter/main.tsx}"
"$(dirname "$0")/mac-sync.sh"
ssh macbook "cd ~/nd && export PATH=\"/etc/profiles/per-user/kyandesutter/bin:\$PATH\" \
  && zig build libnd -Dbackend=abi 2>&1 | tail -3 \
  && cd swift && swift build -c release 2>&1 | tail -5 \
  && cd ~/nd && ND_SCRIPT='$SCRIPT' swift/.build/release/NDShell 2>&1 | tail -20"
```

- [ ] Verify (window opens over ssh, driven by the real core; counter child connects + first commit presents):

```bash
./scripts/mac/mac-sync.sh
ssh macbook 'cd ~/nd && export PATH="/etc/profiles/per-user/kyandesutter/bin:$PATH" \
  && zig build libnd -Dbackend=abi 2>&1 | tail -3 \
  && cd swift && swift build -c release 2>&1 | tail -5'
# Then a bounded headful run: expect ND_COMMIT_APPLIED in stderr, then it holds a window.
ssh macbook 'cd ~/nd && export PATH="/etc/profiles/per-user/kyandesutter/bin:$PATH" \
  && ND_SCRIPT=examples/counter/main.tsx timeout 8s swift/.build/release/NDShell 2>&1 | tail -15'
```

Expected: `swift build` succeeds (links `libnd.a`, resolves every `nd_*` symbol — the `84b58e9` export-retention fix guarantees they're present in the archive); the bounded run prints `ND_COMMIT_APPLIED` (the Bun child handshook and the first commit reached the stub backend). A window opens in the uid-502 GUI session.

**Commit:**
```bash
git add swift/Package.swift swift/Sources/CNd/module.modulemap swift/Sources/CNd/shim.h swift/Sources/NDShell/main.swift scripts/mac/mac-run.sh
git commit -m "feat(swift): swiftpm shell skeleton linking libnd.a"
```

### Interfaces (produced by this task)
- `swift/Package.swift` + `CNd` module map — the SwiftPM package linking `libnd.a` + `include/nd.h`.
- `swift/.build/release/NDShell` — an `NSApplication` that fills a stub `nd_backend`, runs the real core, presents a window over ssh.
- `scripts/mac/mac-run.sh` — the sync→build→run-headful dev loop primitive.

---

## TASK 2 — Codegen Swift emitter (D6)

**Spine (islandable — see Parallelism note). Depends on: M6a codegen (landed). Files: `tools/codegen.ts` (add `genSwift` + a `writeIfChanged` for `swift/Sources/NDGen/Widgets.swift`).**

Extend `tools/codegen.ts` with a `genSwift(schema)` emitter mirroring `genZig`'s structure exactly: a create dispatcher, an `applyProps` dispatcher, `connectEvents`, and the structural ops — each with the **same fail-loud-on-untemplated-widget throws** the Zig emitter uses (`no create template for widget ${w.name}`, `no applyProps template for …`, `no structural template for container widget …`, `no signal template for event …`). Regenerate; the existing generated files' bytes stay unchanged (Swift is a new output path).

- [ ] Add the Swift helper preamble (peer of `ZIG_HELPERS`) + the create dispatcher. Mirror `genZig`'s `if/else if` chain over `s.widgets` in schema order, throwing on any widget without a template:

```typescript
const HEADER_SWIFT = "// GENERATED by tools/codegen.ts — do not edit\n";

const SWIFT_HELPERS = `import AppKit
import Foundation

// JSON-string -> [String: Any] (M6b-D1: JSONSerialization, not a typed decoder).
func parseProps(_ json: String) -> [String: Any] {
    guard let data = json.data(using: .utf8),
          let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
    else { return [:] }
    return obj
}
func propStr(_ p: [String: Any], _ k: String) -> String? { p[k] as? String }
func propInt(_ p: [String: Any], _ k: String) -> Int? { (p[k] as? NSNumber)?.intValue }
func propDouble(_ p: [String: Any], _ k: String) -> Double? { (p[k] as? NSNumber)?.doubleValue }
func propBool(_ p: [String: Any], _ k: String) -> Bool? { (p[k] as? NSNumber)?.boolValue }
func propArray(_ p: [String: Any], _ k: String) -> [String]? { (p[k] as? [Any])?.compactMap { $0 as? String } }

// Every container view class is flipped (top-left y-down) — GLOBAL CONSTRAINT.
final class FlippedView: NSView { override var isFlipped: Bool { true } }
`;

function genSwift(s: Schema): string {
  let out = HEADER_SWIFT + SWIFT_HELPERS + "\n";
  out += "func ndCreate(_ kind: String, _ propsJson: String) -> NSView? {\n";
  out += "    let props = parseProps(propsJson)\n";
  for (let i = 0; i < s.widgets.length; i++) {
    const w = s.widgets[i]!;
    const kw = i === 0 ? "if" : "} else if";
    out += `    ${kw} kind == ${JSON.stringify(w.name)} {\n`;
    out += genSwiftCreateBody(w);
  }
  out += "    }\n";
  out += "    FileHandle.standardError.write(\"ND_WARN unknown widget kind=\\(kind)\\n\".data(using: .utf8)!)\n";
  out += "    return nil\n";
  out += "}\n\n";
  out += genSwiftApplyProps(s);
  out += genSwiftEvents(s);
  out += genSwiftStructural(s);
  return out;
}
```

- [ ] Fill `genSwiftCreateBody(w)` — the per-widget AppKit constructor, one arm per schema widget (mirrors `genZigCreateBody`'s per-`w.name` switch). Each arm reads props with the helpers and returns the native view. The **AppKit mapping table** (locked by T3's decisions; the emitter is where the mapping is realized) is:

| Widget | AppKit class | Notes |
|---|---|---|
| Window | `NSWindow` + flipped `contentView` (`FlippedView`) | `create` returns the `contentView` as the tracked handle; window kept in `gWindow`; `title`/`defaultWidth`/`defaultHeight` at create |
| Box | `NSStackView` | `.orientation` from `orientation`, `.spacing` from `spacing` (M6b-D3) |
| Label | `NSTextField(labelWithString:)` | non-editable, non-bordered |
| Button | `NSButton(title:target:action:)` | target/action wired in `connectEvents` |
| TextInput | `NSTextField` | `placeholder`/`isEditable`; `text` at create+update |
| TextArea | `NSTextView` inside an `NSScrollView` | tracked handle is the `NSScrollView` |
| Checkbox | `NSButton(checkboxWithTitle:target:action:)` | `.state` from `checked` |
| Radio | `NSButton(radioButtonWithTitle:target:action:)` | grouped by shared `action:` per `group` (radios with the same action + superview auto-exclude) |
| Select | `NSPopUpButton(frame:pullsDown:false)` | `addItems(withTitles:)` from `options`; `selectItem(at:)` |
| Slider | `NSSlider` | `minValue`/`maxValue`/`doubleValue`; `.isVertical` from orientation |
| ProgressBar | `NSProgressIndicator` (`.bar`, determinate) | `doubleValue` = `fraction * 100` |
| Image | `NSImageView` | `image = NSImage(contentsOfFile:)` from `path`; `NSImage(named:)` from `iconName` |
| ScrollView | `NSScrollView` with a flipped `documentView` (`FlippedView`) | `hasVerticalScroller = true`; `minContentHeight` → frame min height |
| Separator | `NSBox(.separator)` | `.boxType = .separator`; orientation via frame aspect |
| Spinner | `NSProgressIndicator` (`.spinning`, indeterminate) | `startAnimation`/`stopAnimation` from `spinning` |
| TabView | `NSTabView` | pages added as `NSTabViewItem` in structural ops |
| Grid | `NSGridView` | `attachedProps` gridRow/gridColumn/spans place children |
| ListView | `NSScrollView` + single-column `NSTableView` (view-based recycling) | `itemCount`-not-rows in getTree (M6b-D2) |
| WebView | stub `NSTextField(labelWithString: "WebView unavailable (v1 stub)")` | mirrors the GTK WebView stub; prints the same `ND_WARN` |

Example arms (Window, Box, Button, ListView) showing the shape:

```typescript
function genSwiftCreateBody(w: Widget): string {
  let out = "";
  if (w.name === "Window") {
    out += "        let content = FlippedView()\n";
    out += `        let win = NSWindow(contentRect: NSRect(x: 0, y: 0, width: ${dflt(w, "defaultWidth")}, height: ${dflt(w, "defaultHeight")}), styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)\n`;
    out += "        if let t = propStr(props, \"title\") { win.title = t }\n";
    out += "        win.contentView = content\n";
    out += "        win.center(); win.makeKeyAndOrderFront(nil)\n";
    out += "        gWindow = win\n";
    out += "        return content\n";
  } else if (w.name === "Box") {
    out += "        let stack = NSStackView()\n";
    out += "        let vertical = (propStr(props, \"orientation\") ?? \"vertical\") == \"vertical\"\n";
    out += "        stack.orientation = vertical ? .vertical : .horizontal\n";
    out += `        stack.spacing = CGFloat(propInt(props, "spacing") ?? ${dflt(w, "spacing")})\n`;
    out += "        stack.alignment = vertical ? .leading : .centerY\n";
    out += "        return stack\n";
  } else if (w.name === "Button") {
    out += `        let b = NSButton(title: propStr(props, "label") ?? ${zigDefaultStr(w, "label")}, target: nil, action: nil)\n`;
    out += "        b.setButtonType(.momentaryPushIn); b.bezelStyle = .rounded\n";
    out += "        return b\n";
  } else if (w.name === "ListView") {
    out += "        return makeListView(props)  // NSScrollView+NSTableView, view-based recycling (M6b-D2)\n";
  } else if (/* … one arm per remaining widget … */ false) {
    // Label, TextInput, TextArea, Checkbox, Radio, Select, Slider, ProgressBar,
    // Image, ScrollView, Separator, Spinner, TabView, Grid, WebView — each per
    // the mapping table above, mirroring genZigCreateBody one-for-one.
  } else {
    throw new Error(`no create template for widget ${w.name} — add one when introducing it (M6b)`);
  }
  return out;
}
```

- [ ] Fill `genSwiftApplyProps(s)` — the `createAndUpdate`-prop update dispatcher, mirroring `genZigApplyBody`'s per-`w.name`.`p.name` switch, throwing on any untemplated pair (`no applyProps template for ${w.name}.${p.name}`). Each update includes the **echo-suppression value-equality guard** (the AppKit peer of the GTK `blockEcho`/`unblockEcho` + "only set if different" pattern — see T4), e.g. `TextInput.text` only calls `stringValue = t` when `field.stringValue != t`.

- [ ] Fill `genSwiftEvents(s)` — a `SIGNALS`-equivalent table mapping each schema event to a target/action selector, throwing on any event without a template (`no signal template for event ${w.name}.${e.name}`). Since AppKit uses target/action (not GObject signals), `connectEvents(view, kind, nodeID)` sets `view.target`/`view.action` to a dispatcher object that calls `nd_emit_event(gCtx, nodeID, name, payloadJson)`. The generated `connectEvents` mirrors `genZigEvents`'s per-kind `if/else if` chain; the emitted callback bodies build the JSON payload per `payload` kind (`none`/`text`/`checked`/`value`/`index`), matching `CALLBACK_BODIES`.

- [ ] Fill `genSwiftStructural(s)` — `appendChild`/`insertBefore`/`removeChild` over the containers (`s.widgets.filter(w => w.container !== null)`), throwing on any container without a structural template (`no structural template for container widget ${w.name}`). Mirror `STRUCTURAL`: Window→`contentView`-single-child; Box→`NSStackView.addArrangedSubview`/`insertArrangedSubview(at:)`/`removeArrangedSubview`; ScrollView→`documentView` single child; TabView→`NSTabViewItem` add/insert/remove (with `tabLabel` attached prop); Grid→`NSGridView` cell placement by `gridRow`/`gridColumn`/spans.

- [ ] Register the new output at the bottom of `codegen.ts` (next to the existing `writeIfChanged` calls):

```typescript
await writeIfChanged("swift/Sources/NDGen/Widgets.swift", genSwift(schema));
```

- [ ] Verify (Swift emitted; existing generated bytes unchanged; freshness gate green):

```bash
nix develop -c bash -c 'bun tools/codegen.ts \
  && git diff --exit-code -- packages/react/src/generated src/generated docs/widgets.md docs/styling.md \
  && test -f swift/Sources/NDGen/Widgets.swift \
  && head -1 swift/Sources/NDGen/Widgets.swift | grep -q "GENERATED by tools/codegen.ts"'
```

Expected: `Widgets.swift` written with the generated header; the existing Zig/TS/docs generated files are byte-identical (the `git diff --exit-code` passes). If a schema widget lacks a template, `bun tools/codegen.ts` throws loudly — the same fail-loud contract as the Zig emitter.

**Commit:**
```bash
git add tools/codegen.ts swift/Sources/NDGen/Widgets.swift
git commit -m "feat(codegen): swift widget emitter (D6)"
```

### Interfaces (produced by this task)
- `genSwift(schema)` in `tools/codegen.ts` — the D6 Swift emitter, fail-loud parity with `genZig`.
- `swift/Sources/NDGen/Widgets.swift` — `ndCreate`/`ndApplyProps`/`ndConnectEvents`/`ndAppendChild`/`ndInsertBefore`/`ndRemoveChild`, committed + deterministic.

---

## TASK 3 — AppKit backend: layout + the 18 widgets

**Spine. Depends on: T1, T2. Files: `swift/Sources/NDShell/Backend.swift` (NEW), `swift/Sources/NDShell/main.swift` (wire the real vtable), `swift/Sources/NDGen/ListView.swift` (NEW — the `NSTableView` data source, hand-written host code the emitter references via `makeListView`).**

Fill the `nd_backend` vtable in Swift from `Widgets.swift`, the AppKit peer of `src/gtk/backend.zig`'s vtable fill. Each `vt*` C-callconv closure casts the `nd_widget` handle back to `NSView`, decodes the JSON arg, and calls the generated function — never a narrower concrete type (the generated dispatcher owns per-kind casts, same as GTK).

- [ ] `swift/Sources/NDShell/Backend.swift` — the structural + prop vtable fills, mirroring `vtCreate`/`vtApplyProps`/`vtAppendChild`/… in `src/gtk/backend.zig`. Handles cross as retained `Unmanaged` pointers (the core holds them for the node's lifetime; unparent/remove balance the retain):

```swift
import AppKit
import CNd
import NDGen  // the generated ndCreate/ndApplyProps/… + FlippedView

var gWindow: NSWindow?

@inline(__always) func viewFrom(_ p: UnsafeMutableRawPointer?) -> NSView {
    Unmanaged<NSView>.fromOpaque(p!).takeUnretainedValue()
}
@inline(__always) func cstr(_ p: UnsafePointer<CChar>?) -> String {
    p.map { String(cString: $0) } ?? ""
}

func buildVTable() -> nd_backend {
    var vt = nd_backend()
    vt.create = { _, kind, propsJson in
        guard let v = ndCreate(cstr(kind), cstr(propsJson)) else { return nil }
        return Unmanaged.passRetained(v).toOpaque()   // core owns the handle
    }
    vt.apply_props = { _, w, kind, propsJson in
        ndApplyProps(viewFrom(w), cstr(kind), cstr(propsJson))
    }
    vt.append_child = { _, parent, pkind, child, attachedJson in
        ndAppendChild(viewFrom(parent), cstr(pkind), viewFrom(child), cstr(attachedJson))
    }
    vt.insert_before = { _, parent, pkind, child, before, attachedJson in
        let beforeView = before.map { Unmanaged<NSView>.fromOpaque($0).takeUnretainedValue() }
        ndInsertBefore(viewFrom(parent), cstr(pkind), viewFrom(child), beforeView, cstr(attachedJson))
    }
    vt.remove_child = { _, parent, pkind, child in
        ndRemoveChild(viewFrom(parent), cstr(pkind), viewFrom(child))
    }
    vt.set_text = { _, w, text in
        if let f = viewFrom(w) as? NSTextField { f.stringValue = cstr(text) }
    }
    vt.set_visible = { _, w, visible in viewFrom(w).isHidden = !visible }
    vt.apply_style = { _, w, nodeID, styleJson in
        ndApplyStyle(viewFrom(w), nodeID, cstr(styleJson))  // T3-lite: NSColor/NSFont/frame insets
    }
    vt.connect_events = { _, w, kind, nodeID in ndConnectEvents(viewFrom(w), cstr(kind), nodeID) }
    vt.has_parent = { _, w in viewFrom(w).superview != nil }
    vt.unparent = { _, w in viewFrom(w).removeFromSuperview() }
    vt.get_window = { _ in gWindow?.contentView.map { Unmanaged.passUnretained($0).toOpaque() } }
    vt.marshal_async = { _, fn, data in DispatchQueue.main.async { fn?(data) } }
    vt.show_overlay = { _, message in ndShowOverlay(cstr(message)) }  // T4: crash overlay chrome
    // node_visible / node_bounds / snapshot / semantic_action filled in T5.
    return vt
}
```

- [ ] `ndApplyStyle(view, nodeID, styleJson)` — the AppKit style applier (the peer of GTK's `style.applyStyle`; **AppKit styling is `NSColor`/`NSFont`/frame insets, NOT CSS** — M6a-D5's reason `style.zig` stayed GTK-only). Decode the `style` object (`background`→`layer.backgroundColor` with `wantsLayer`; `color`/`font`→per-control; `padding`/`margin`→frame insets; `border`→`layer.borderWidth`/`borderColor`/`cornerRadius`). Unknown keys are already rejected React-side; this is defensive.

- [ ] `swift/Sources/NDGen/ListView.swift` — the hand-written `makeListView(props)` + `NSTableViewDataSource`/`Delegate` the emitter references (M6b-D2). Single column, view-based recycling over `items: [String]`; the tracked handle is the outer `NSScrollView`. Store the `items` array + `selectedIndex` on the data source; `ndApplyProps` for `ListView.items`/`selectedIndex` reloads. `getTree`'s `itemCount` reads `items.count` (T5's `node_bounds`/tree walk exposes it via the core, which already reports `itemCount` for `childModel: null` list widgets from `Tree.meta`).

- [ ] Wire the real vtable in `main.swift`: replace `buildStubVTable()` with `buildVTable()`.

- [ ] Verify (counter renders + increments over ssh — the button and the `Clicks:` label present in a non-blank screenshot, using T5's fidelity solution once T5 lands; at T3 assert render via `getTree`):

```bash
./scripts/mac/mac-sync.sh
ssh macbook 'cd ~/nd && export PATH="/etc/profiles/per-user/kyandesutter/bin:$PATH" \
  && zig build libnd -Dbackend=abi 2>&1 | tail -3 \
  && cd swift && swift build -c release 2>&1 | tail -5 \
  && cd ~/nd && ND_SCRIPT=examples/counter/main.tsx NATIVE_AUTOMATION=1 timeout 10s swift/.build/release/NDShell 2>&1 | tail -15'
```

Expected: `ND_AUTOMATION_LISTENING path=…` + `ND_COMMIT_APPLIED` in stderr; the counter window shows an `Increment` button + `Clicks: 0` label (full non-blank-screenshot proof lands with T5/T6).

**Commit:**
```bash
git add swift/Sources/NDShell/Backend.swift swift/Sources/NDShell/main.swift swift/Sources/NDGen/ListView.swift
git commit -m "feat(appkit): 18-widget backend + isFlipped layout"
```

### Interfaces (produced by this task)
- `buildVTable()` fills the structural/prop/style half of `nd_backend` from `NDGen.Widgets`; handles ride as retained `Unmanaged<NSView>`.
- The 18 widgets render natively (Box→`NSStackView`, ListView→`NSTableView`, window `contentView`/ScrollView `documentView` flipped).

---

## TASK 4 — Events up + controlled-widget echo suppression

**Spine. Depends on: T3. Files: `swift/Sources/NDGen/Widgets.swift` (via codegen — the event callbacks), `swift/Sources/NDShell/Events.swift` (NEW — the target/action dispatcher + suppression state), `swift/Sources/NDShell/Overlay.swift` (NEW — `ndShowOverlay`).**

Swift control targets/actions call `nd_emit_event(gCtx, nodeID, name, payloadJson)` (embedder→core, the peer of GTK's `emitEventAdapter` → `nd_emit_event`). Port the M5b echo-suppression: a controlled input whose value React just set must not emit a `changed` event back (which would loop). AppKit has no signal-block API, so suppression is a **per-view value-equality guard + a re-entrancy flag** (the peer of GTK's `blockEcho`/`unblockEcho`).

- [ ] `swift/Sources/NDShell/Events.swift` — the target/action dispatcher object (`connectEvents` sets `view.target = EventDispatcher.shared` / `view.action = #selector(...)`) and the suppression registry. The dispatcher reads the sender's current value, builds the payload JSON, and calls `nd_emit_event`. A `suppressed: Set<ObjectIdentifier>` set is consulted before emitting; `ndApplyProps`'s update path adds the view to `suppressed` around the mutation:

```swift
import AppKit
import CNd

final class EventDispatcher: NSObject {
    static let shared = EventDispatcher()
    // View -> the node_id + event name to emit (set by ndConnectEvents).
    var wiring: [ObjectIdentifier: (nodeID: UInt32, name: String, payload: PayloadKind)] = [:]
    var suppressed: Set<ObjectIdentifier> = []   // don't echo a React-driven mutation

    @objc func fire(_ sender: NSControl) {
        let key = ObjectIdentifier(sender)
        guard !suppressed.contains(key), let w = wiring[key] else { return }
        let json = payloadJSON(sender, w.payload)   // {"text":…}/{"checked":…}/{"value":…}/{"index":…}/{}
        nd_emit_event(gCtx, w.nodeID, w.name, json)
    }
}

// Called by the generated ndApplyProps update guards: mutate without echoing.
func withEchoSuppressed(_ view: NSView, _ body: () -> Void) {
    let key = ObjectIdentifier(view)
    EventDispatcher.shared.suppressed.insert(key)
    body()
    EventDispatcher.shared.suppressed.remove(key)
}
```

- [ ] The generated `ndApplyProps` update arms (T2) wrap controlled-value mutations in `withEchoSuppressed` AND the value-equality guard, mirroring the GTK generated pattern (`if cur != t { blockEcho; set; unblockEcho }`). Example the emitter produces for `TextInput.text`:

```swift
if let t = propStr(props, "text"), let f = view as? NSTextField, f.stringValue != t {
    withEchoSuppressed(view) { f.stringValue = t }
}
```

(`Checkbox.checked`/`Radio.checked`/`Select.selectedIndex`/`Slider.value`/`TextArea.text` get the same guard, one-for-one with `genZigApplyBody`.)

- [ ] `swift/Sources/NDShell/Overlay.swift` — `ndShowOverlay(message)`, the peer of GTK's `vtShowOverlay`. Empty `message` clears; non-empty shows a crash-overlay `NSView` over the window `contentView` (a semi-opaque `FlippedView` with a centered `NSTextField` + a Restart `NSButton` gated on `ND_DEV=1`, whose action emits the reserved `nd_emit_event(gCtx, 0, "restart", "{}")` sentinel — same contract as `src/gtk/backend.zig:246`'s `onRestartIdle`). This is backend-specific chrome; the core calls it via `vtable.show_overlay`.

- [ ] Verify (counter `onClick` and a TextField `onChanged` round-trip through React over ssh):

```bash
./scripts/mac/mac-sync.sh
ssh macbook 'cd ~/nd && export PATH="/etc/profiles/per-user/kyandesutter/bin:$PATH" \
  && zig build libnd -Dbackend=abi 2>&1 | tail -3 && cd swift && swift build -c release 2>&1 | tail -5 \
  && cd ~/nd && ND_SCRIPT=examples/counter/main.tsx NATIVE_AUTOMATION=1 timeout 12s swift/.build/release/NDShell 2>&1 | tail -20'
```

Expected: after the first commit, stderr shows the React round-trip (`ND_COMMIT_APPLIED` repeats — the uptime interval drives commits, and a click would advance `Clicks:`). The full click-through assertion is T6's drive script; T4's gate is that the shell builds with events wired and no echo loop hangs it.

**Commit:**
```bash
git add swift/Sources/NDShell/Events.swift swift/Sources/NDShell/Overlay.swift tools/codegen.ts swift/Sources/NDGen/Widgets.swift
git commit -m "feat(appkit): events up + echo suppression"
```

### Interfaces (produced by this task)
- `EventDispatcher` (target/action → `nd_emit_event`) + `withEchoSuppressed` — controlled inputs round-trip without looping.
- `ndShowOverlay` — the crash overlay + `ND_DEV` Restart sentinel, backend chrome behind `vtable.show_overlay`.

---

## TASK 5 — TCC-free automation backend + the screenshot fidelity ladder

**Spine. Depends on: T4. Files: `swift/Sources/NDShell/Automation.swift` (NEW — `node_visible`/`node_bounds`/`snapshot`/`semantic_action` fills), `swift/Sources/NDShell/Backend.swift` (wire them + testIDs).**

Fill the automation half of the vtable — the AppKit peer of `src/gtk/backend.zig`'s `vtNodeVisible`/`vtNodeBounds`/`vtSnapshot`/`vtSemanticAction`. The automation **server** (socket, framing, `waitFor`, SLO) is core-owned (M6a-D3) and already answers on Mac once these callbacks exist; the SLO holds because these ops are fast main-thread work marshaled via `dispatch_async_f`, never touching the Bun child. **This task carries M6a's one pre-identified risk: naive `cacheDisplay` returns blank subviews** — the fidelity ladder (M6b-D4) is here.

- [ ] `node_visible` + `node_bounds` (peers of `vtNodeVisible`/`vtNodeBounds`). `visible` = `!view.isHidden && view.window != nil` (mapped folds into visible, the v1 contract from M6a Task 4). `bounds` = the view's frame converted to window-content-view space, in `nd_rect` (top-left y-down — the `FlippedView` roots make this direct):

```swift
func vtNodeVisible(_ ctx: OpaquePointer?, _ w: UnsafeMutableRawPointer?) -> Bool {
    let v = viewFrom(w)
    return !v.isHidden && v.window != nil
}
func vtNodeBounds(_ ctx: OpaquePointer?, _ w: UnsafeMutableRawPointer?, _ out: UnsafeMutablePointer<nd_rect>?) -> Bool {
    let v = viewFrom(w)
    guard let content = gWindow?.contentView else { return false }
    let r = v.convert(v.bounds, to: content)   // content is flipped -> top-left space
    out?.pointee = nd_rect(x: Int32(r.origin.x), y: Int32(r.origin.y), w: Int32(r.width), h: Int32(r.height))
    return true
}
```

- [ ] `snapshot` — the **fidelity ladder** (M6b-D4), ordered rungs, first non-blank wins. `non-blank ≡ the PNG bitmap has >1 distinct pixel colour`. Wire the chosen rung index into the result JSON:

```swift
func vtSnapshot(_ ctx: OpaquePointer?, _ pngPath: UnsafePointer<CChar>?) -> Bool {
    guard let content = gWindow?.contentView else { return false }
    let path = String(cString: pngPath!)
    let bounds = content.bounds
    // Rung 1: force display, then cacheDisplay.
    content.displayIfNeeded()
    if let rep = content.bitmapImageRepForCachingDisplay(in: bounds) {
        content.cacheDisplay(in: bounds, to: rep)
        if writeIfNonBlank(rep, path) { return true }
    }
    // Rung 2: layer-back the tree, render the CALayer into a CGContext bitmap.
    setWantsLayerRecursive(content)
    if let rep = renderLayerBitmap(content, bounds), writeIfNonBlank(rep, path) { return true }
    // Rung 3: per-view lockFocus composite.
    if let rep = compositeLockFocus(content, bounds), writeIfNonBlank(rep, path) { return true }
    // Rung 4: PDF -> raster.
    if let rep = pdfRasterize(content, bounds), writeIfNonBlank(rep, path) { return true }
    return false   // core answers the RPC with -32603 (empty snapshot), same as GTK
}

// >1 distinct pixel colour == non-blank (the exact acceptance criterion).
func writeIfNonBlank(_ rep: NSBitmapImageRep, _ path: String) -> Bool {
    guard hasMoreThanOneColor(rep),
          let png = rep.representation(using: .png, properties: [:]) else { return false }
    do { try png.write(to: URL(fileURLWithPath: path)); return true } catch { return false }
}
```

(`setWantsLayerRecursive`/`renderLayerBitmap` (CALayer.render(in:)), `compositeLockFocus` (`NSBitmapImageRep(focusingViewRect:)` per mapped subview), `pdfRasterize` (`dataWithPDF(inside:)` → `NSImage` → bitmap), and `hasMoreThanOneColor` are the ladder's rung helpers. **`CGWindowListCreateImage` is FORBIDDEN — TCC.** The driver's 150ms→3s non-blank poll (T6) wraps this so an unready frame retries.)

- [ ] `semantic_action` — dispatch on `action`, peer of `vtSemanticAction` (M6b-D5). `click`→`performClick(nil)`; `setValue`→per-control; `type`→append + fire; `scroll`→`NSScrollView` clip scroll. Mallocs `result_json_out`/`err_json_out` with `malloc` so the core's `nd_free` (libc `free`) releases them uniformly:

```swift
func vtSemanticAction(_ ctx: OpaquePointer?, _ w: UnsafeMutableRawPointer?, _ nodeID: UInt32,
                      _ action: UnsafePointer<CChar>?, _ argJson: UnsafePointer<CChar>?,
                      _ resultOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?,
                      _ errOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> Int32 {
    let v = viewFrom(w); let act = String(cString: action!); let args = parseProps(String(cString: argJson!))
    switch act {
    case "click":
        (v as? NSControl)?.performClick(nil)
        setResult(resultOut, ["ref": nodeID, "dispatched": true]); return 0
    case "setValue":  return semanticSetValue(v, nodeID, args, resultOut, errOut)
    case "type":      return semanticType(v, nodeID, args, resultOut, errOut)
    case "scroll":    return semanticScroll(v, nodeID, args, resultOut)
    default: setErr(errOut, nodeID); return -32601
    }
}
```

- [ ] testIDs: `apply_props`/`create` set `view.setAccessibilityIdentifier(testID)` and `gWindow?.setAccessibilityIdentifier(...)` — but the `getTree` `testID` field already flows from `Tree.meta` (core-owned, the `meta` prop), so accessibility identifiers are the *native* mirror, not the tree source. Set them for real-user/VoiceOver parity.

- [ ] Wire `vt.node_visible`/`node_bounds`/`snapshot`/`semantic_action` into `buildVTable()`.

- [ ] Verify (`getTree`/`screenshot`(non-blank)/`click`/`setValue`/`type`/`scroll` answer over the Mac automation socket; SIGSTOP SLO passes on Mac). Run the counter headful on the Mac with `NATIVE_AUTOMATION=1`, grab the socket path from stderr, and drive it **on the Mac**:

```bash
./scripts/mac/mac-sync.sh
ssh macbook 'cd ~/nd && export PATH="/etc/profiles/per-user/kyandesutter/bin:$PATH" \
  && zig build libnd -Dbackend=abi >/dev/null 2>&1 && cd swift && swift build -c release 2>&1 | tail -3 \
  && cd ~/nd && ND_SCRIPT=examples/counter/main.tsx NATIVE_AUTOMATION=1 swift/.build/release/NDShell >/tmp/nd.log 2>&1 & \
  for _ in $(seq 1 80); do grep -q ND_AUTOMATION_LISTENING /tmp/nd.log && grep -q ND_COMMIT_APPLIED /tmp/nd.log && break; sleep 0.1; done; \
  SOCK=$(grep -m1 ND_AUTOMATION_LISTENING /tmp/nd.log | sed "s/.*path=//"); \
  ND_AUTOMATION_SOCKET="$SOCK" ND_SHOT_PATH=/tmp/m6-shot.png bun scripts/m6-drive.ts 2>&1 | tail -5; \
  file /tmp/m6-shot.png'
```

Expected: `M6_DRIVE_OK clicks=3 …` + `/tmp/m6-shot.png: PNG image` with >1 colour (the fidelity ladder produced a non-blank shot; the result JSON records which rung). The SLO leg (T6's `mac-m6.sh`) SIGSTOPs the Bun child and confirms `getTree`+`screenshot` still answer under 5s.

**Commit:**
```bash
git add swift/Sources/NDShell/Automation.swift swift/Sources/NDShell/Backend.swift
git commit -m "feat(appkit): tcc-free automation + screenshot fidelity ladder"
```

### Interfaces (produced by this task)
- `node_visible`/`node_bounds`/`snapshot`(non-blank via the 4-rung ladder)/`semantic_action` fill the automation half of `nd_backend`; the core's automation server answers over the Mac socket; the D11 SLO holds (backend ops are main-thread-fast).

---

## TASK 6 — Mac headful drive scripts + the demos

**Spine. Depends on: T5. Files: `scripts/m6-drive.ts` (NEW), `scripts/mac/mac-m6.sh` (NEW). Reuses `examples/counter/main.tsx` + `examples/gallery/main.tsx` unchanged.**

`scripts/mac/mac-m6.sh` mirrors `scripts/headless-m4.sh` + `scripts/headless-m5c.sh`, but **headful-in-session on the Mac** (uid `$(id -u)`=502, `GUI_OK`): it syncs, builds, launches the shell with `NATIVE_AUTOMATION=1`, waits for the same `ND_AUTOMATION_LISTENING`/`ND_COMMIT_APPLIED` markers, drives the counter + gallery via `bun scripts/m6-drive.ts` **on the Mac against the local socket**, scps the screenshots back for the orchestrator to view, and runs the SIGSTOP-SLO leg. No `ssh -L` tunnel — the driver runs remotely, exactly like `mac-build.sh` runs `zig build` remotely.

- [ ] `scripts/m6-drive.ts` — reuses `packages/mcp/src/socket.ts`'s `AutomationClient` (the same class `m4-drive.ts`/`m5c-drive.ts` import). Two legs, selected by `argv`: `counter` (getTree→find `increment-button`→click×3→waitFor `Clicks: 3`→non-blank screenshot with the 150ms→3s poll) and `gallery` (styled tab + widget assertions, mirroring `m5c-drive.ts`), plus `--slo` (getTree+screenshot only, for the stalled-child leg). The **screenshot poll mirrors `m5c-drive.ts` exactly** (retry up to 20× at 150ms) since the fidelity ladder may need a settled frame:

```typescript
#!/usr/bin/env bun
// scripts/m6-drive.ts — drives the Mac AppKit shell over the automation socket.
// Runs ON THE MAC (mac-m6.sh ssh's in and invokes it against the local socket).
// Reuses the same AutomationClient as m4/m5c; assertions mirror them.
import { AutomationClient } from "../packages/mcp/src/socket.ts";

interface TreeNode { ref: number; type: string; testID: string | null; text: string | null;
  visible: boolean; geometry: { x: number; y: number; w: number; h: number } | null;
  children: TreeNode[]; itemCount?: number | null; }
interface GetTreeResult { coordinateSpace: string; root: TreeNode; }

function find(n: TreeNode, id: string): TreeNode | null {
  if (n.testID === id) return n;
  for (const c of n.children) { const f = find(c, id); if (f) return f; }
  return null;
}
async function pollScreenshot(client: AutomationClient, path: string) {
  let shot: any = null, lastErr: Error | null = null;
  for (let i = 0; i < 20; i++) {
    try { shot = await client.call("screenshot", { path }); break; }
    catch (e) { lastErr = e as Error; await new Promise((r) => setTimeout(r, 150)); }
  }
  if (!shot) throw new Error(`screenshot failed after retries: ${lastErr?.message}`);
  if (shot.width <= 0 || shot.height <= 0) throw new Error("screenshot has no dimensions");
  return shot;
}

const mode = process.argv[2] ?? "counter";
const slo = process.argv.includes("--slo");
const outPng = process.env.ND_SHOT_PATH ?? "/tmp/m6-shot.png";
const client = await AutomationClient.connect();

if (slo) {
  const tree = (await client.call("getTree")) as GetTreeResult;
  if (tree.coordinateSpace !== "logical-window-topleft") throw new Error("bad getTree under stall");
  const shot = await pollScreenshot(client, outPng);
  console.log(`M6_SLO_OK tree+screenshot answered while child stalled png=${shot.path} ${shot.width}x${shot.height}`);
  client.close(); process.exit(0);
}

if (mode === "counter") {
  const tree = (await client.call("getTree")) as GetTreeResult;
  if (tree.coordinateSpace !== "logical-window-topleft") throw new Error("bad coordinate space");
  const btn = find(tree.root, "increment-button");
  if (!btn) throw new Error("increment-button not found");
  for (let i = 0; i < 3; i++) {
    const res = (await client.call("click", { ref: btn.ref })) as { dispatched: boolean };
    if (!res.dispatched) throw new Error(`click ${i + 1} did not dispatch`);
  }
  const waited = (await client.call("waitFor", { condition: { textContains: "Clicks: 3" }, timeoutMs: 3000 })) as { matched: boolean };
  if (!waited.matched) throw new Error("waitFor Clicks: 3 did not match");
  const shot = await pollScreenshot(client, outPng);
  console.log(`M6_DRIVE_OK clicks=3 png=${shot.path} ${shot.width}x${shot.height} rung=${shot.rung}`);
} else if (mode === "gallery") {
  const tree = (await client.call("getTree")) as GetTreeResult;
  const styledTab = find(tree.root, "styled-tab"); if (styledTab?.type !== "Box") throw new Error("styled-tab wrong");
  const styledButton = find(tree.root, "styled-button"); if (styledButton?.type !== "Button") throw new Error("styled-button wrong");
  const list = find(tree.root, "big-list");
  if (list?.type !== "ListView") throw new Error("big-list not a ListView");
  if (list.itemCount !== 100000) throw new Error(`itemCount=${list.itemCount}, want 100000`);
  if (list.children.length !== 0) throw new Error(`ListView dumped ${list.children.length} children`);
  const shot = await pollScreenshot(client, outPng);
  console.log(`M6_GALLERY_OK styled+list png=${shot.path} ${shot.width}x${shot.height} itemCount=${list.itemCount}`);
}
client.close();
```

- [ ] `scripts/mac/mac-m6.sh` — the headful-in-session orchestrator, mirroring `headless-m4.sh`'s launch/wait/drive/SLO structure but over ssh with the profile PATH. Runs the counter + gallery legs, scps both PNGs back, and runs the SIGSTOP-SLO leg (find the Bun child via `pgrep -P`, `kill -STOP`, drive `--slo` under `timeout 5`, `kill -CONT`):

```bash
#!/usr/bin/env bash
set -euo pipefail
"$(dirname "$0")/mac-sync.sh"

# Build + drive entirely on the Mac; scp screenshots back here afterwards.
ssh macbook 'bash -euo pipefail -s' <<'REMOTE'
export PATH="/etc/profiles/per-user/kyandesutter/bin:$PATH"
cd ~/nd
zig build libnd -Dbackend=abi >/dev/null 2>&1
(cd swift && swift build -c release >/dev/null 2>&1)

run_leg() {  # $1=script $2=mode $3=out.png
  ND_SCRIPT="$1" NATIVE_AUTOMATION=1 swift/.build/release/NDShell >/tmp/nd-$2.log 2>&1 &
  local PID=$!
  for _ in $(seq 1 100); do
    grep -q ND_AUTOMATION_LISTENING /tmp/nd-$2.log && grep -q ND_COMMIT_APPLIED /tmp/nd-$2.log && break; sleep 0.1
  done
  local SOCK; SOCK=$(grep -m1 ND_AUTOMATION_LISTENING /tmp/nd-$2.log | sed 's/.*path=//')
  ND_AUTOMATION_SOCKET="$SOCK" ND_SHOT_PATH="$3" bun scripts/m6-drive.ts "$2" >/tmp/drive-$2.log 2>&1 \
    || { echo "FAIL drive $2"; cat /tmp/drive-$2.log; cat /tmp/nd-$2.log; kill "$PID" 2>/dev/null; exit 1; }
  cat /tmp/drive-$2.log
  file "$3" | grep -q "PNG image" || { echo "FAIL $2 not a png"; kill "$PID" 2>/dev/null; exit 1; }

  if [ "$2" = "counter" ]; then   # D11 SLO leg: stall the bun child, automation must answer <5s.
    local BUN; BUN=$(pgrep -P "$PID" -f bun | head -1)
    [ -n "$BUN" ] || { echo "FAIL no bun child for SLO"; kill "$PID" 2>/dev/null; exit 1; }
    kill -STOP "$BUN"
    ND_AUTOMATION_SOCKET="$SOCK" ND_SHOT_PATH=/tmp/m6-slo.png timeout 5 bun scripts/m6-drive.ts --slo >/tmp/slo.log 2>&1 || { kill -CONT "$BUN"; echo "FAIL SLO >5s"; cat /tmp/slo.log; kill "$PID" 2>/dev/null; exit 1; }
    kill -CONT "$BUN"; cat /tmp/slo.log; grep -q M6_SLO_OK /tmp/slo.log || { echo "FAIL SLO no marker"; exit 1; }
  fi
  kill -TERM "$PID" 2>/dev/null || true; wait "$PID" 2>/dev/null || true
}

run_leg examples/counter/main.tsx counter /tmp/m6-counter.png
run_leg examples/gallery/main.tsx gallery /tmp/m6-gallery.png
echo "MAC_M6_OK"
REMOTE

# Bring the screenshots back for the orchestrator to view.
scp macbook:/tmp/m6-counter.png macbook:/tmp/m6-gallery.png /tmp/ 2>/dev/null || true
echo "MAC_M6_SCREENSHOTS /tmp/m6-counter.png /tmp/m6-gallery.png"
```

- [ ] Verify:

```bash
./scripts/mac/mac-m6.sh 2>&1 | tail -30
file /tmp/m6-counter.png /tmp/m6-gallery.png
```

Expected: `M6_DRIVE_OK clicks=3 … rung=N`, `M6_SLO_OK …`, `M6_GALLERY_OK … itemCount=100000`, then `MAC_M6_OK`; both scp'd PNGs are `PNG image` and non-blank. The MCP server drives the Mac app identically to Linux (same `AutomationClient`, same RPC methods).

**Commit:**
```bash
git add scripts/m6-drive.ts scripts/mac/mac-m6.sh
git commit -m "feat(mac): headful drive scripts + counter/gallery demos"
```

### Interfaces (produced by this task)
- `scripts/mac/mac-m6.sh` + `scripts/m6-drive.ts` — headful-in-session Mac acceptance: counter + gallery driven over the automation socket, non-blank PNGs returned via scp, SIGSTOP-SLO green on Mac.

---

## TASK 7 (STRETCH, optional, END) — GitHub Actions macOS runner job

**Islandable, non-blocking. Depends on: T6 green. Files: `.github/workflows/mac.yml` (NEW). Must NOT block the ssh dev loop or the Linux gate.**

A headful macOS-runner job running the SwiftPM build + a `mac-m6`-equivalent drive in a logged-in session. The GitHub remote exists (`git@github.com:FormalSnake/NativeDesktop.git`), so this is viable; per the M6 research, stock `macos-latest` runners run headful and the TCC-free design needs no screen-recording grant. Keep it **optional** — if runner setup churns (SDK/Swift version skew, the runner's non-GUI session breaking `NSApplication`), document-and-defer rather than block.

- [ ] `.github/workflows/mac.yml` — a `macos-latest` job: checkout, install the Zig pin (0.16.0) + Bun pin (1.3.13), `zig build libnd -Dbackend=abi`, `swift build -c release` in `swift/`, then run a CI variant of the counter drive (the `run_leg counter` body, adapted to the runner's session). Gate: `continue-on-error: true` at the job level OR marked non-required in branch protection — a red mac job must never block a merge.

```yaml
name: mac
on: [push, pull_request]
jobs:
  macos-appkit:
    runs-on: macos-latest
    continue-on-error: true   # STRETCH: never blocks the Linux gate or a merge.
    steps:
      - uses: actions/checkout@v4
      - name: Install Zig 0.16.0
        run: |  # mise/asdf or a direct download pinned to 0.16.0
          curl -sL https://ziglang.org/download/0.16.0/zig-macos-aarch64-0.16.0.tar.xz | tar xJ
          echo "$PWD/zig-macos-aarch64-0.16.0" >> "$GITHUB_PATH"
      - uses: oven-sh/setup-bun@v2
        with: { bun-version: 1.3.13 }
      - name: Build libnd + Swift shell
        run: |
          zig build libnd -Dbackend=abi
          (cd swift && swift build -c release)
      - name: Drive the counter (headful, TCC-free)
        run: |
          ND_SCRIPT=examples/counter/main.tsx NATIVE_AUTOMATION=1 swift/.build/release/NDShell >/tmp/nd.log 2>&1 &
          for _ in $(seq 1 100); do grep -q ND_AUTOMATION_LISTENING /tmp/nd.log && grep -q ND_COMMIT_APPLIED /tmp/nd.log && break; sleep 0.1; done
          SOCK=$(grep -m1 ND_AUTOMATION_LISTENING /tmp/nd.log | sed 's/.*path=//')
          ND_AUTOMATION_SOCKET="$SOCK" ND_SHOT_PATH=/tmp/ci-shot.png bun scripts/m6-drive.ts counter
      - uses: actions/upload-artifact@v4
        with: { name: mac-counter-shot, path: /tmp/ci-shot.png }
```

- [ ] Verify (push the branch; the mac job runs). If it stays red on runner-session issues, capture the failure, mark the task **documented-and-deferred** in the self-review, and leave the workflow with `continue-on-error: true` (or delete it if it can't even build) — do NOT block M6b on it.

```bash
gh run list --workflow=mac.yml --limit 1
gh run view --log --job=macos-appkit | tail -40
```

Expected: the mac job is green (build + counter drive + uploaded non-blank shot), OR documented-and-deferred with the exact runner error captured. Either outcome is acceptable — this is a stretch.

**Commit:**
```bash
git add .github/workflows/mac.yml
git commit -m "ci(mac): stretch macos-runner appkit build + counter drive (non-blocking)"
```

### Interfaces (produced by this task)
- `.github/workflows/mac.yml` — an optional, non-blocking macOS CI job proving the SwiftPM build + TCC-free counter drive run on a stock runner, or a documented deferral.

---

## Parallelism note (M6b)

M6b is **mostly a spine with one honest fan-out at the front.** Unlike M6a (strict spine, every task edited `include/nd.h`), M6b's ABI is frozen — so **T1 (SwiftPM skeleton) and T2 (codegen Swift emitter) are independent islands** and CAN run in parallel:

- **T1** only writes `swift/Package.swift` + the `CNd` module map + a stub `main.swift` + `mac-run.sh`. It links `libnd.a` and needs nothing from the emitter (its backend is a hand-stubbed vtable).
- **T2** only edits `tools/codegen.ts` + emits `swift/Sources/NDGen/Widgets.swift`. It is a pure codegen change, verified by the Linux freshness gate — no Mac, no `libnd`.

Dispatch T1 and T2 to two subagents concurrently. **T3 needs both** (it wires the real `buildVTable()` from `NDGen.Widgets`), so it is the join point; from T3 onward the plan is a strict spine (T3→T4→T5→T6), because each builds on the prior Swift backend surface. **T7 is a detached island** — it depends only on T6 being green and must never block anything.

**Linux vs Mac work:** T2 is authored + verified entirely on **this Linux box** (the freshness gate). T1/T3/T4/T5/T6 build the Swift shell **on the Mac over ssh** (`mac-sync.sh` → `swift build`), and drive it headful-in-session there — this Linux box orchestrates via `ssh macbook` and receives screenshots via `scp`. The **whole Linux gate must stay green** after T2's codegen change (the existing generated bytes must not shift — that is T2's verify).

---

## Self-review (plan-level)

- **ABI fidelity:** every `nd_backend` field is filled with an AppKit closure that mirrors `src/gtk/backend.zig`'s corresponding `vt*` one-for-one (structural ops, `marshal_async`→`dispatch_async_f`, `show_overlay`, `node_visible`/`node_bounds`, `snapshot`, `semantic_action`). No new vtable field is invented — `include/nd.h` is frozen (M6a-D1), and the Swift side re-parses the same NUL-terminated JSON the Zig side stringifies (M6a-D2). Cross-checked field names/order against `include/nd.h` (create/apply_props/append_child/insert_before/remove_child/set_text/set_visible/apply_style/connect_events/has_parent/unparent/get_window/marshal_async/show_overlay/node_visible/node_bounds/snapshot/semantic_action) and `semantic_action`'s `result_json_out`/`err_json_out` malloc-and-`nd_free` contract.
- **`isFlipped` everywhere:** `FlippedView` overrides `isFlipped`; it roots the window `contentView` and every `ScrollView.documentView` (M6b-D3). `NSStackView` positions arranged subviews by constraints, so it lays out correctly under flipped ancestors. The self-review greps that no bare `NSView` is used as a container class.
- **Main-thread + no deadlock:** `NSApplication.run()` on the main thread; `marshal_async` uses `DispatchQueue.main.async` (`dispatch_async_f` semantics), never `dispatch_sync` from main. Automation ops marshal to main, so the D11 SLO stays a core property (the automation thread never touches the Bun child) — re-verified on Mac in T5/T6.
- **The carried risk is contained:** the screenshot fidelity ladder (M6b-D4) is a concrete, ordered 4-rung fallback with a stated `>1 distinct pixel colour` non-blank gate per rung, plus a 150ms→3s driver poll — the exact shape the M6a probe row demanded. `CGWindowListCreateImage` is forbidden (TCC).
- **Fail-loud codegen parity:** `genSwift` throws on any untemplated widget/prop/event/container, identical to `genZig`'s throws — a new schema widget breaks `bun tools/codegen.ts` loudly on both backends.
- **No scope creep:** no `include/nd.h` change, no schema change, no React/TS-renderer change, WebView stays a stub. The Linux gate stays byte-identical (T2's only risk, gated).

---

## Landed-code cross-references (authoritative, `file:line`)

- GTK vtable fill to mirror: `src/gtk/backend.zig:524` `ndBackend()` (every field), `:150` `vtCreate`, `:226` `vtMarshalAsync` (the `invokeFull` peer of `dispatch_async_f`), `:261` `vtShowOverlay`, `:278` `vtNodeVisible`, `:286` `vtNodeBounds`, `:309` `vtSnapshot`, `:350` `vtSemanticAction`, `:398`–`:520` the click/setValue/type/scroll bodies.
- GTK embedder main to mirror: `src/gtk/main.zig:20` `main` (lifecycle order), `:18` `the_vtable` (must outlive the frame — the `gVTable` module-scope lesson).
- Codegen structure to mirror: `tools/codegen.ts:278` `genZig` (dispatcher shape), `:366` `genZigCreateBody` (per-widget arms + the untemplated-throw at `:492`), `:497` `genZigApplyBody` (echo-suppression guards), `:697` `genZigEvents` (SIGNALS + throw), `:796` `genZigStructural` (STRUCTURAL + throw), `:909` `writeIfChanged` + the emit block at `:915`.
- Build/link: `build.zig:193` `libnd` static lib (`-Dbackend=abi`, GTK-free) + `:200` `include/nd.h` install; `src/core/root.zig:26` the comptime export-retention block (guarantees `nd_*` symbols are in `libnd.a` for the Swift link).
- Automation drive scaffolds to mirror: `scripts/m4-drive.ts` (counter click×3 + SLO), `scripts/m5c-drive.ts` (gallery + `itemCount` + the 150ms→3s screenshot poll), `packages/mcp/src/socket.ts` `AutomationClient` (`ND_AUTOMATION_SOCKET`, `getTree`/`click`/`waitFor`/`screenshot`/`scroll`).
- Mac dev-loop primitives to build on: `scripts/mac/mac-sync.sh` (bare repo + `mac` remote), `scripts/mac/mac-build.sh` (profile PATH, no nix, run-on-Mac-over-ssh).
