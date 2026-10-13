# Adversarial review: ARCHITECTURE.md and docs/plan/examples

Reviewed 2026-10-13 against `docs/plan/PROMPT.md`, `docs/plan/ARCHITECTURE.md` (line numbers below
are into that file unless a path is given), `docs/plan/examples/**`, `docs/research/*.md`,
`docs/parity/*.md`, the `zig` branch of the old repo, and zed at `60b0983f` (the commit
`docs/research/gpui.md` pins; `Z/` below is
`https://github.com/zed-industries/zed/blob/60b0983f189fb183ba3c95f44e0a7ae6aba8c640/`).
Ranked most severe first. Severity: S1 blocks the design, S2 breaks a stated requirement or gate,
S3 costs real rework if left.

## S1

### 1. On the AppKit backend there is no GPUI `Window`, so actions, key contexts and focus have nothing to run on
What: 3.1 (l.93-99) says `App`, `Entity`, `Context`, focus handles and the keymap are thin newtypes
over GPUI, "so the semantics are GPUI's". In GPUI the dispatch tree that `on_action`,
`key_context` and focus paths resolve against lives in `Window::rendered_frame` and is rebuilt
while GPUI paints elements. `FocusHandle::focus` takes `&mut gpui::Window`. The AppKit backend never
paints GPUI elements, so every `.on_action(..)`, `.key_context(..)`, `KeyBinding` context,
`window.dispatch_action` and `focus_handle().focus(..)` in the examples has no tree to dispatch on.
The design is also silent on who owns `NSApplication`: `gpui::Application` on macOS brings
`gpui_macos`'s own app class, delegate, menus and windows, and nd's `NSWindow`/`NSMenu` would fight
it. 3.7(5) (l.318) "focus synced both ways" assumes a GPUI focus system that does not exist there.
The result is two dispatch implementations (GPUI's on Linux/Windows/web, something unwritten on
macOS) that will drift.
Evidence: `Z/crates/gpui/src/window.rs:612` (`focus(&self, window: &mut Window, cx: &mut App)`),
`:639-647` and `:2546-2557` (dispatch resolves through `rendered_frame.dispatch_tree`), `:1043`;
`Z/crates/gpui/src/platform.rs:363` (`pub trait Platform`, so a custom platform is possible).
Fix: nd owns focus, key contexts, keystroke matching and action dispatch over its retained tree
on every backend, using GPUI's `Keymap` only as a matcher. GPUI backends forward raw key events
into that dispatcher instead of using GPUI's per-frame tree. On macOS write an `NdAppKitPlatform`
implementing `gpui::Platform` (run loop, dispatcher, menus, open URLs) so nd, not `gpui_macos`,
owns `NSApplication`. Make this the first M0 lane and spike it before any other lane starts; the
key view loop across `NSToolbar` (a separate view hierarchy) and recycled table cells belongs in
the same spike.

### 2. AppKit callbacks re-enter the app synchronously while the reconciler holds the entity
What: AppKit fires delegate and data-source callbacks inside setters and layout: `setRegion`
fires `regionDidChange`, `selectRowIndexes` fires `selectionDidChange`, `reloadData`/scrolling call
`viewForTableColumn:row:` and `heightOfRow`, `NSSplitView` resizes call delegates mid-layout.
The architecture routes the row callbacks straight into app code (`list(&state,
cx.processor(Self::row))`, 7 l.577-580) and native widget events through `Emit` with no stated
queueing. In-process (unlike the zig two-process model, where every event was async by
construction) these land while the view or `NoteList` entity is leased, and GPUI panics on a
double lease ("cannot update ... while it is already being updated"). Nested run loops make it
worse: `NSMenu popUpMenu`, `runModal`, `performDrag`/dragging sessions spin a loop inside the app's
handler, so timers, tasks and the automation server run re-entrantly under the same borrow.
Evidence: `examples/native-widget/src/map_view/appkit.rs:41` sets the region inside `update`, and
`:65-71` emits from `regionDidChange`; `examples/notes/src/note_list.rs:121`; ARCHITECTURE l.577-582.
Fix: one rule in `nd-ui`: no native callback calls app code. Emits go into a per-window queue
drained after `apply` and after the run loop returns from any nested loop. Table data sources
answer from a row-element cache that `nd-virtual` filled during the last render pass (rows for the
viewport plus overscan are rendered ahead, outside AppKit callbacks). Menus, alerts and drags are
started from a deferred main-queue hop, never inside a borrow. Add a tier 2 test that sets every
native widget's props from inside its own event handler.

### 3. Controlled native widgets have no echo contract, and the examples already loop
What: the vocabulary is controlled (app owns value, widget emits change, app feeds it back), but
nothing says what happens when the native control normalises the value. `MKMapView` adjusts any
region to its aspect ratio, so `RegionChanged` returns a different span, `zoom_for_span` returns a
different zoom, `update` sees `o.zoom != new.zoom` and calls `setRegion(animated: true)` again:
an endless animated loop. The same class hits text fields (setting `stringValue` mid-composition
destroys IME marked text and the caret), sliders, split dividers and `NSTextView` undo
(two undo stacks: the entity's and AppKit's). The notes editor shows the text case:
`Editor::open` calls `set_text`, which emits `TextEvent::Changed`, which calls `set_body`, which
bumps `updated`, so merely opening a note moves it to the top of the list.
Evidence: `examples/native-widget/src/map_view/appkit.rs:38-41,99-101`;
`examples/notes/src/editor.rs:16-21,29`, `examples/notes/src/model.rs:139-147`; the zig branch
already had to learn this for panes (`docs/parity/zig.md:129,332`).
Fix: a per-prop sync contract on `NativeWidget`: the backend records the last value the native
side reported and skips writes equal to it; events caused by a programmatic set are suppressed
(generation counter); text state is native-owned while focused, with `Changed` fired only for
user edits and a separate `Replaced` for `set_text`. Put an echo test in the widget conformance
suite.

### 4. Reconciling by position destroys native views whenever a `.when()` flips
What: 3.7 (l.288) reconciles "by position and `.id()` key". GPUI-style code builds children with
`.when(..)`/`.when_some(..)`, which leave no hole, so a conditional child appearing before a native
view shifts every later sibling. The reconciler then sees a type change and tears down and
recreates the `NSTextView`, `WKWebView`, `MKMapView` or CEF view: lost focus, IME state, page
state, scroll position. The zig branch fixed exactly this in its renderer.
Evidence: ARCHITECTURE l.288-290; `docs/parity/zig.md:335` ("emptied expression refills in own
slot, not after neighbours").
Fix: identity is `(element type, explicit id)` if present, else `(type, #[track_caller]
call-site, occurrence index)`. Stateful native widgets (`webview`, `text_editor`, `map_view`,
`list`) require an id at the type level (their constructors already take one; make `text_editor`
and `search_field` match). Add a tier 1 test that toggles a `.when` above a text field and asserts
the backend saw no create.

## S2

### 5. `NSTableView` cannot honour the virtualization guarantees as designed
What: 7 (l.579-580, 596) has heights "from Taffy measurement of the row at table width through
`heightOfRow`" plus "estimates, idle-time measurement". `heightOfRow` is called by AppKit, not by
us, and since macOS 13 AppKit estimates unmeasured rows itself from the ones it measured, with no
API to hand it our estimate. Measuring in `heightOfRow` means running app row code inside an AppKit
callback (finding 2) and, without AppKit's estimation, for all 100k rows on reload. With AppKit's
estimation, the content height and the anchor drift as AppKit corrects its own estimates, which is
the "last rows disappearing" class on macOS. The one conformance suite "runs against both
presenters" (l.601-603), but tier 1 has only the `TestBackend`, so the AppKit presenter is never
run under it.
Evidence: Apple AppKit release notes for macOS 13 (view-based tables estimate row heights);
`NSTableViewCanEstimateRowHeights` opt-out
([christiantietze.de](https://christiantietze.de/posts/2022/11/nstableview-variable-row-heights-broken-macos-ventura-13-0));
ARCHITECTURE l.583-585, 941.
Fix: `nd-virtual` owns the height tree on every backend. On macOS disable AppKit estimation, answer
`heightOfRow` in O(1) from that tree (measured or our estimate), measure near the viewport outside
callbacks, then `noteHeightOfRowsWithIndexesChanged` and shift the clip origin in one transaction.
Run the conformance suite on a real `NSTableView` in tier 2 (manual run-loop stepping,
animations off) and name that in the M5 gate.

### 6. GPUI owns the OS connections that `nd-os-*` plans to use; the patch queue is much larger than stated
What: `nd-os-linux` depends on `wayland-client` (l.56) to add `pointer-gestures`,
`wp_presentation`, `xdg-toplevel-drag`, KDE blur and `xdg_popup` menus, but GPUI owns the
`wl_display`, registry and event queue, so those objects must be bound inside `gpui_linux`.
`nd-os-windows` needs DWM frame extension, `WM_POINTER`, DirectManipulation and per-monitor
clocks, all inside GPUI's wndproc and swap chain. AccessKit: GPUI already creates an adapter per
window (`gpui.md:87-91`), so nd "feeding AccessKit" (3.7(6), l.321) would be a second producer for
the same window. 2.1 (l.82-88) lists six patches and calls it "small"; the real list is roughly
twice that, plus youtubemusic's GL-path patch (`gpui.md:26-29`), and two of them (window
composition #62379, glass content #58833) are open upstream PRs.
Evidence: ARCHITECTURE l.56-57, 82-88, 321, 546-547, 1005; `docs/research/gpui.md:24-29,87-91,192-216`.
Fix: write the complete patch list in 2.1 with an owner and an upstream PR per patch, and treat
`nativedesktop/zed` as a maintained fork from M0 with a rebase gate. Expose nd semantics to GPUI's
own AccessKit integration through element roles instead of a second adapter. `nd-os-*` crates
become patch consumers, not protocol owners.

### 7. The Swift toolchain and SDK in the Nix shell cannot build the macOS backend
What: 3.9 (l.362-367) says Command Line Tools are enough and "the Nix dev shell provides it on
darwin". Measured on the macbook today: inside the project's Nix shell `xcrun --show-sdk-version`
reports 14.4 (nix apple-sdk), nixpkgs `swift` is 6.2.4, while `/usr/bin/swift` is 6.4 with the
macOS 27 SDK. The design needs 26/27 SDK APIs (`NSGlassEffectView`, glass toolbar and sidebar
APIs, newest SwiftUI). Building Swift with the system toolchain (`env -u SDKROOT`, the old quirk)
and then linking through Nix's clang/cctools against SDK 14.4 mixes SDKs: Swift objects reference
SwiftUI/AppKit symbols the 14.4 `.tbd` files do not export.
Evidence: `nix eval nixpkgs#swift.version` = 6.2.4; `xcrun --show-sdk-version` in the dev shell =
14.4; `/usr/bin/swift --version` = 6.4, target macOS 27.0; ARCHITECTURE l.362-367, 1009.
Fix: on darwin the dev shell does not export `SDKROOT` or a linker; `.cargo/config.toml` sets
`target.aarch64-apple-darwin.linker = "/usr/bin/clang"` and the build script resolves the SDK with
`xcrun --sdk macosx --show-sdk-path`. `nd doctor` checks that Swift and the SDK major match.
`mkApp` builds for darwin need the same impurity (`__noChroot` or a prebuilt Swift archive).

### 8. Linking Swift from library build scripts cannot work as described
What: four separate breakages in 3.9's bridge:
(a) `cargo:rustc-link-arg` (needed for the Swift rpath and `-force_load`) applies only to the
package's own binaries and tests, never to dependents, so `nd-backend-appkit`'s build.rs cannot
set them for an app, and `examples/hello` has no build.rs at all.
(b) Rust finds Swift widgets by string (`SwiftWidget::named("Gauge")`), so nothing references the
`@_cdecl` registration symbol and the linker drops that member of the static archive.
(c) `libNDSwift.a` and every app or crate package (`swift/Widgets`) each compile `NDKit` into
their own static library: duplicate symbols, or two copies of `NDSwiftWidget`'s protocol metadata
so the conformance lookup fails.
(d) `nd::build` "materializes" `NDKit` into `../../.nd/NDKit` inside the source tree, which breaks
`cargo publish` verification and races between worktrees.
Also `Gauge.swift` omits `measure`, which the `NDSwiftWidget` contract in l.375 requires, and its
`NDEmitter<Never>` does not match the Rust `type Event = ()`.
Evidence: Cargo build-script docs for `rustc-link-arg` (applies to the package's own targets);
`examples/native-widget/build.rs:3`, `swift/Widgets/Package.swift:8`,
`Sources/Widgets/Gauge.swift:14-29,44`; ARCHITECTURE l.362-381.
Fix: each crate with Swift declares `links = "nd-swift-<name>"` and exports its package path as
`DEP_*` metadata. The app's own build.rs (`nd::build::app()`, emitted by `nd new` into every
template) generates one aggregate SwiftPM package in `OUT_DIR` depending on the framework package
and all collected crate packages, builds one static library, and emits `-force_load`, the rpath
and the Swift library search paths for the final binary. Registration is a generated Rust
`extern "C"` table naming every exported symbol, so nothing depends on dead-strip behaviour.

### 9. The e1504g target has no budget and no gate until M5, and the GPUI path does whole-window work
What: 3.7 (l.291) promises "cost is the size of the dirty view, never the window". On upstream
GPUI any notify re-runs Taffy over the window and re-renders the notified views; only gpui-fast
skips that, and here it is an optional feature (l.299-300). nd adds a second tree on GPUI
backends (retained nd tree, then GPUI elements every frame). The frame recorder copies every
presented frame into a ring (9.2), about 8 MB per 1080p frame on an iGPU that shares single-channel
memory. The only e1504g gate is the M5 100k-row scroll; "measured from M2" (l.1011) has no
numbers, though the prompt makes e1504g snappiness the target and research asked for week-one
measurement (`gpui.md:331`).
Evidence: ARCHITECTURE l.12, 291, 298-300, 636-638, 1011; `docs/research/gpui.md:255-271`.
Fix: numeric budgets in section 9 and gates from M0 (GPUI side) and M2 on e1504g, for example:
cold start to first frame, idle CPU at 0.0% over 60 s, RSS with the GL path patched out,
key-to-present p95 under one 60 Hz interval measured by `wp_presentation`, zero missed frames
scrolling 10k rows. Make gpui-fast's retained mode the Linux default, or drop the intermediate
element tree on GPUI backends. Keep only a downscaled last-frame copy in production and the full
ring under test.

### 10. Milestone order has hard contradictions that block parallel lanes
What: (a) the M4 gate runs "overlay leak test over a native surface on three OSes" (l.936) but
`NativeSurface`, WKWebView, WebView2 and WPE arrive in M8 (l.954). (b) M1 moves all
`docs/plan/examples` into `examples/` (l.915), and l.795-797 compiles `examples/` in CI, but those
apps use `scaffold`, `list`, `settings_form`, `command_palette`, `glass` and `Store` from M3-M7, so
CI is red from M1 to M7. (c) M0 splits into three lanes (l.911) before the seam between `nd-ui`
and backends exists (finding 11), so the AppKit lane has nothing to code against.
Evidence: ARCHITECTURE l.795-797, 911, 915, 936, 954.
Fix: pull `NativeSurface` plus a minimal WKWebView and a WebView2 composition host into M4 (or
move the leak gate to M8). Move examples in M1 but compile each behind the milestone that ships
its APIs (a `required-features` list per example). Make "backend seam spec + `TestBackend`" an M0
day-one task that lanes start from.

### 11. The seam between the reconciler and the backends is never specified
What: the zig branch had a written op set (create, append, insertBefore, remove, setText, update,
hide, unhide), ownership rules, ordering ("updates ship after inserts/removals so a popover opens
over its new content") and a null backend with a conformance suite. The new design mentions a
`TestBackend` that "records backend ops" (l.662-663) and `NativeWidget` hooks, but no op list,
no ordering rule, no ownership or threading rule, no frame/transaction boundary, and no statement
of how the AppKit and GPUI backends consume the same ops.
Evidence: ARCHITECTURE l.286-291, 662-663, 840; `docs/parity/zig.md:185,189,197,328`.
Fix: add a section defining the `Backend` trait: ops, ordering, node lifetime and release,
layout boundaries, the transaction boundary, and which ops may arrive during live resize. Port
the zig conformance suite against it.

### 12. The resize gate does not test live resize or presented pixels
What: M0's `--nd-selftest resize` "drives 50 live-resize steps" (l.908-910). A programmatic
`setFrame` never enters live resize (`inLiveResize` stays false, no `viewWillStartLiveResize`), so
the path the prompt complains about is untested. Asserting "frame size equals window size" reads
back the frames the backend just set in the same transaction, which is true by construction; the
failure users see is a presented frame that does not match, which only pixels show. 5.2 (l.499-500)
repeats the same check for all OSes.
Evidence: PROMPT l.30; ARCHITECTURE l.493-500, 908-910.
Fix: drive a real edge drag (`ndshell` HID CGEvents on macOS, virtual pointer on headless
Hyprland, `SendInput` on Windows) and capture each presented frame (SCStream, `wp_presentation` +
`ext-image-copy-capture`, Windows.Graphics.Capture), asserting content edges against the window
edge per frame, with a webview and a GPU surface in the window.

### 13. Frame recording and animation capture do not exist for AppKit-drawn windows
What: 9.2 (l.636-638) records "Metal drawable copies and `CALayer` snapshots" on macOS. An AppKit
window has no app-owned Metal drawable, and `renderInContext` misses `NSVisualEffectView`,
`NSGlassEffectView`, `CAMetalLayer`, WKWebView and anything the render server composites.
ScreenCaptureKit needs a TCC grant shipped apps do not have, so incident bundles on user machines
get no frames. Tier 2 animation capture by `advance_clock` (l.668-669) cannot step Core Animation,
which runs in the render server, yet 3.7(4) (l.316-317) maps `.with_animation` to Core Animation.
The prompt's "what UI layers there are" is answered only by the semantic tree, which omits native
views, layers and the overlay stack.
Evidence: ARCHITECTURE l.316-317, 636-638, 645, 666-669; PROMPT l.46, 52.
Fix: on macOS, bundles record per-frame layout and tree diffs (bounds, z-order, overlay stack,
native view hierarchy) instead of pixels. Pixel recording happens only under `ndshell` with SCStream.
Animation tests freeze Core Animation with `layer.speed = 0` and step `timeOffset` per frame.

### 14. Windows tier 3 over `ssh windows` cannot show windows or inject input
What: processes started by OpenSSH on Windows run outside the interactive desktop, `SendInput`
reaches only the foreground input desktop, and Windows.Graphics.Capture needs a visible window.
The Windows PC is also the user's gaming machine, so real-input tests would type into a game.
Evidence: ARCHITECTURE l.686; `docs/research/testing-profiling.md:133-139`; PROMPT l.24.
Fix: a resident `ndshell agent` started at logon in the user's session (Task Scheduler,
interactive), which `nd test` reaches over ssh or a named pipe; it refuses real-input jobs while a
fullscreen app or game is foreground, and runs them on a second session (RDP loopback user) when
one is configured.

### 15. Concurrent agents collide on macOS real-input tests
What: 10.3 (l.692-693) serialises macOS tier 3 with a nextest `max-threads = 1` group. That
serialises within one `cargo nextest` run only; two agents in two worktrees, the exact scenario in
the prompt, still post HID events and steal activation from each other.
Evidence: PROMPT l.52; ARCHITECTURE l.689-693.
Fix: a machine-wide lock (flock under `~/Library/Caches/nd/`) taken by `ndshell` for HID input,
activation and SCStream; NSEvent-queue tests skip the lock and stay parallel. Report the holder's
worktree when waiting.

### 16. macOS custom panels as child `NSPanel`s break key status and close themselves
What: 6.3 (l.559-561) makes custom panels child `NSPanel`s and closes overlays "on key-window
loss" (l.566). A panel with a text field must become key, which deactivates the parent window's
chrome (grey traffic lights, inactive toolbar) and triggers the key-loss rule on its own owner.
That is the "floating thing that doesn't feel part of the window" complaint. The zig palette, which
3.11 ports "exactly", is an in-window view over the frame view, not a panel.
Evidence: ARCHITECTURE l.420, 559-566; PROMPT l.30; `docs/research/macos-and-cef.md:367`.
Fix: content-anchored overlays are a top-most in-window view (above WKWebView, CEF and GPU surface
views, which are ordinary sibling NSViews). Use a non-activating `NSPanel` only when the overlay
must extend past the window frame. Dismiss on app deactivation or a click outside, not on key
change within the app's own window group.

### 17. CEF's IOSurface cannot be used as `CALayer` contents
What: 8 (l.615) and the risk table (l.999) show the OSR IOSurface directly as layer contents to
avoid a GPU surface. CEF documents that the handle's resource "cannot be cached" and must be
copied into a client-owned texture; holding it as layer contents displays a surface Chromium is
already reusing.
Evidence: `docs/research/macos-and-cef.md:261-263`; ARCHITECTURE l.615, 999.
Fix: blit each `OnAcceleratedPaint` into an IOSurface from our own pool (Metal blit) and set that
as contents, double-buffered. Budget the Metal path on macOS from M9.

### 18. nativebrowser on Hyprland loses extensions, 1Password and native messaging
What: Chrome-style windowed CEF is planned for "macOS, Windows and X11" (l.618-620). The user's
Linux desktop is Hyprland on Wayland, and GPUI runs there as a Wayland client, so the Chrome-style
browser has nothing to embed into. The zig branch shipped CEF on Hyprland through XWayland with
Hyprland-specific handling for Chromium windows.
Evidence: ARCHITECTURE l.618-620; `docs/parity/zig.md:211,220`; old repo `src/cef/hyprland.zig`
(branch `zig`); CEF Wayland windowed is unshipped (`macos-and-cef.md:251`).
Fix: decide in writing: nativebrowser forces GPUI's X11 backend under XWayland when
`style = "chrome"`, or extensions stay on OSR via the extension APIs the zig branch used. Gate M15
on extensions and 1Password working on Hyprland.

### 19. The examples do not type-check against GPUI's context model or against each other
What:
- ARCHITECTURE 3.2 (l.132-141): `note` is `Option<&Note>` used as a `Note`; `tag_color` does not
  exist (`model.rs:15-23` has `tag: Option<Tag>`); the `move` canvas closure captures a borrow of
  `cx` and is not `'static`. l.123 calls `visible_ids()` with no arguments (`model.rs:80` takes
  two). l.174 calls `editor.open(e.key, cx)` (`editor.rs:26` takes `id, window, cx`).
- `notes/src/app.rs:75`: `self.editor.read(cx).focus(window, cx)` holds `&App` from `read` while
  passing `&mut App` (E0502).
- `app.rs:102`, `editor.rs:34`: `focus_handle().focus(window)`; GPUI's signature is
  `focus(window, cx)` (`Z/crates/gpui/src/window.rs:612`).
- `cx.processor(Self::row)` with `row(&mut self, id: &NoteId, ..)` (`note_list.rs:46,121`,
  `sidebar.rs:24,34`, `native-widget/src/main.rs:35,52`): GPUI's `processor` returns
  `impl Fn(E, ..) -> R + 'static` (`Z/crates/gpui/src/app/context.rs:260-263`), which forces
  `E = &'static NoteId`.
Evidence: as listed.
Fix: fix 3.2 to match `examples/notes`; rows take `K: Copy` by value, or nd defines its own
`processor` with an HRTB signature; run `cargo check` on the examples against a stub `nd` crate in
M0 so the design target compiles before any implementation exists.

### 20. The example apps contradict the framework's own rules
What:
- `notes/tests/notes.rs:55` expects completion `"lis|bon"`, but the palette contract only shows a
  completion that extends the typed text (l.437-438) and `app.rs:135` sets the completion to the
  full title "Trip to Lisbon"; "Release checklist" also matches `lis`.
- `app.rs:197-206`: `on_submit` sets `palette_open = false` without `cx.notify()`, so the
  controlled palette stays open.
- `settings.rs:7` puts `deny_unknown_fields` on the root settings struct while 3.4 (l.209-216)
  merges plugin sections such as `discord` into the same file, so enabling a plugin makes the
  config fail to parse.
- `overrides/src/main.rs:53-54,84,88` use colour literals that `nd check` denies (l.893), and
  `:30` hand-tunes `.ml(px(72.))` past the traffic lights, the per-app pixel tuning the non-goals
  forbid (l.35-36).
Evidence: as listed.
Fix: correct the examples, give plugin sections their own namespace (`[plugins.discord]`) with
unknown-key checks per section, and run `nd check` over `examples/` in CI.

### 21. Custom titlebars cannot reserve space where the OS puts its controls
What: `window_controls().at(point(..))` pins coordinates and `.embedded()` makes a slot, but nothing
tells content where the OS's own controls are: traffic lights on the left on macOS, caption
buttons on the right on Windows, either side on GNOME (`button-layout`), none on Hyprland. The
overrides example therefore leaves a 72 px gap on Hyprland and puts its Share button under the
Windows caption buttons.
Evidence: `examples/overrides/src/main.rs:23-41`; ARCHITECTURE l.513-524; PROMPT l.54.
Fix: a `titlebar()` intent that exposes leading and trailing insets from the platform (side and
size, updated on fullscreen and RTL), with `window_controls()` resolving to the OS side
automatically; lint px margins placed beside `window_controls`.

### 22. The Windows caption decision contradicts the research it cites
What: 3.10 (l.395-399) and 5.4 have the system draw caption buttons through
`DwmExtendFrameIntoClientArea`/`DwmDefWindowProc`. The Windows research recommends app-drawn
buttons with `HTMAXBUTTON` hit-testing, which is what GPUI does today. DWM buttons have a fixed
height that cannot follow a taller Fluent title row, and GPUI's DirectComposition swap chain and
caption hit-test code would have to change.
Evidence: `docs/research/windows-packaging-updates.md:91-94,244`; ARCHITECTURE l.395-399, 520-522.
Fix: spike both in M2 on the Windows host; unless DWM buttons pass a tall-titlebar and Snap
Layouts test, follow the research and record the exception to 3.10.

### 23. Fluent goldens cannot be compared to WinUI Gallery off Windows
What: M3 compares Fluent goldens to WinUI Gallery references (l.930), but tier 2 renders the
Fluent look headless with Metal or wgpu (l.665-666). Segoe UI Variable is not redistributable,
DirectWrite rasterises differently, and Windows headless GPUI is unverified (GPUI's Windows
renderer is DirectX 11).
Evidence: ARCHITECTURE l.665-666, 930; `docs/research/testing-profiling.md:62`.
Fix: render Fluent goldens on the Windows host only; compare control metrics (sizes, paddings,
radii, sampled colours) against WinUI Gallery, pixels only Windows to Windows.

### 24. Tier 1 cannot prove AppKit behaviour, yet M5 and M6 gate on it
What: tier 1 runs on `TestBackend` (l.662-664). The notes tests for context menus, swipe actions,
sheets and the palette, and M6's "per component tier 1 tests", all pass on that backend while the
`NSMenu`, `NSTableView` row actions and `NSPanel` mappings stay untested. The swipe test also looks
up the "Delete" row action by role, but row action buttons are drawn by AppKit and locators never
query the OS accessibility tree (l.660-661), so that test cannot pass on macOS.
Evidence: `examples/notes/tests/notes.rs:93-101`; ARCHITECTURE l.657-664, 944-947.
Fix: every component gate names a tier 2 AppKit test. Framework-created native items (menu items
from a `MenuSpec`, row actions) are added to the semantic tree as synthetic nodes with real
action routes.

### 25. `swipe_actions` and `context_menu` are element modifiers but map to table-level APIs
What: row swipe actions on macOS come from the table delegate
(`tableView:rowActionsForRow:edge:`), so `.swipe_actions` on anything other than a list row root
cannot be expressed, but the API allows it on any element.
Evidence: ARCHITECTURE l.149-150, 542; `examples/notes/src/note_list.rs:104-113`.
Fix: make swipe actions a method on the list row builder (`list(..).row_actions(|key| ..)`), so
misuse does not compile.

### 26. The web gate proves compilation, not that examples run
What: M11's gate is "every crate in `examples/` builds for wasm32 unchanged" (l.971). On
`wasm32-unknown-unknown`, `std::time::Instant::now`, `SystemTime::now`, threads and `std::fs`
compile and then panic at runtime. `notes` calls `jiff::Timestamp::now()` (`model.rs:104,143`),
which needs jiff's `js` feature; `nd-data` (rusqlite) and `nd::runtime::tokio()` do not run there.
Evidence: ARCHITECTURE l.197-198, 766-768, 971.
Fix: run each example's tier 1 tests under `wasm-bindgen-test` in headless Chromium in the M11
gate, and have `nd check --target web` deny direct `std::time`/`std::thread`/`std::fs` in app crates.

### 27. Framework lints need a lint engine the plan does not name
What: 15.4 (l.890-896) promises framework lints (colour literals, `cfg(target_os)` placement,
`Instant::now` in UI code, hand-rolled scroll lists, overlays not created through `cx.overlay`).
Clippy cannot load custom lints; dylint needs a pinned nightly per lint library. Agent-first
quality rests on these.
Evidence: ARCHITECTURE l.890-896, 43.
Fix: choose now: a `syn`-based checker inside `nd check` (stable, fast, file and line output) for
syntactic rules, and type-aware rules as tier 1 audits at runtime.

### 28. Webview parity is mapped as one row; WPE and WebView2 composition carry most of it
What: the zig webview surface is about 25 events and 30 commands (scheme handlers, script messages,
cookies, find, session save, permissions, downloads, user scripts). Section 8 lists engines and a
few behaviours; the parity row is "Kept". WPE is a new engine on Linux and starts from zero.
WebView2 in composition mode needs mouse, pointer, drag-drop, cursor and UIA forwarded by the host
(`ICoreWebView2CompositionController` `SendMouseInput`, `DragEnter`, `CursorChanged`,
`AutomationProvider`), none of which M8 gates.
Evidence: `docs/parity/zig.md:203-219`; ARCHITECTURE l.605-623, 841, 954-956.
Fix: an engine-by-API matrix in section 8 (WKWebView, WebView2, WPE, CEF OSR, iframe) with a gate per
cell, and WebView2 input forwarding items listed in the M8 gate.

## S3

### 29. Parity items dropped without a row
What: the `@nativedesktop/ui` components (Accordion, ButtonGroup, DescriptionList, Form/FormField,
HoverCard, OtpInput, Pagination, SearchableList, StatusBar, Stepper) appear nowhere in 3.11;
`settings.d` drop-ins from the config pattern (apps.md item 19) are missing from the settings
layers (l.749-750); `clipboard.readImage`; the zig "JS crash leaves the window up" resilience is
replaced by relaunch without saying a hang now freezes the window.
Evidence: `docs/parity/zig.md:165,183,239`; `docs/parity/apps.md` section 5 item 19; ARCHITECTURE
l.405-415, 749-750, 825.
Fix: add explicit rows (port, replace or drop with reason) to section 14.

### 30. HMR is a feature loss with no gate
What: the zig branch had state-preserving component HMR; the replacement is relaunch with state
restore, and `subsecond` is only "evaluated in M10". The prompt says "no true feature loss".
Evidence: `docs/parity/zig.md:194`; ARCHITECTURE l.839, 964, 1014.
Fix: spike `subsecond` against `nd-ui` in M1 (render functions re-run on patch, entities kept) and
set a gate: edit to visible change under 1.5 s with view state kept.

### 31. Cranelift for `nd dev` needs nightly
What: `nd dev` uses Cranelift (l.705-706), which ships only as a nightly rustup component
(`-Zcodegen-backend`); the dev shell provides stable rustc 1.95. Cranelift also aborts on panic on
macOS, which removes the panic-to-bundle path in dev on the main dev machine.
Evidence: ARCHITECTURE l.705-706; rustc_codegen_cranelift README and the 2025H2 project goal
([rust-project-goals](https://rust-lang.github.io/rust-project-goals/2025h2/production-ready-cranelift.html)).
Fix: state the pinned nightly for dev builds, or use LLVM with `opt-level = 0` plus
`split-debuginfo = "unpacked"` and measure both in M0.

### 32. Rows rendered through a processor are not views, so (g) and the dirty-view promise fail
What: requirement (g) says "row subtrees are views; a row animation touches one row" (l.597), but
rows come from `cx.processor(Self::row)` on the list's owner. Any notify of the owner re-runs
every visible row, and `canvas` closures are never `PartialEq`, so every visible row repaints.
In notes each keystroke in the editor notifies the library, `NoteList` re-sorts every note with
two lowercase allocations per note (`model.rs:80-90`), and the sidebar recounts all folders. Agents
will copy these examples as the pattern.
Evidence: ARCHITECTURE l.597; `examples/notes/src/note_list.rs:40-44,121`,
`examples/notes/src/model.rs:80-90`, `examples/notes/src/sidebar.rs:20,25`.
Fix: `nd-virtual` caches each row's element output by `(key, row props hash)` and re-renders only
rows whose inputs changed; `canvas` takes a cache key; the examples keep a sorted index updated
incrementally.

### 33. Embedded GPUI surfaces on macOS each get their own renderer and clip their own popovers
What: each GPU-only subtree on macOS is a separate embedded `PlatformWindow` (l.329-336), so each
gets its own Metal renderer and sprite atlas. GPUI components inside (the code editor's completion
menu, hover cards built with `deferred(anchored(..))`) draw inside the surface and are clipped,
while 3.8 says overlays from inside go through the native path without saying how GPUI-internal
popovers get there.
Evidence: ARCHITECTURE l.327-336, 999.
Fix: one shared renderer and atlas across a window's embedded surfaces, and a GPUI patch that
routes `deferred` overlays from embedded surfaces to nd's overlay layer.

### 34. Inactive windows capped at 30 fps hurt the multi-monitor case
What: 5.3 (l.509) caps inactive windows at 30 fps. On the g815, a visible unfocused window on the
240 Hz or 144 Hz monitor (youtubemusic lyrics, a video) drops to 30.
Evidence: ARCHITECTURE l.509; PROMPT l.46.
Fix: cap only occluded or minimised windows (`NSWindow.occlusionState`, Wayland frame callbacks
already stop, `DXGI_STATUS_OCCLUDED` on Windows); visible windows run at their display's rate
while animating.

### 35. The Windows mixed-refresh gate names hardware the Windows host may not have
What: M2 gates Windows per-monitor pacing on "g815 (240 + 144 Hz)" (l.923), but Windows tests run
on the separate Windows PC over `ssh windows` (l.686). Unless that PC drives a 240 Hz and a 144 Hz
monitor, the Windows pacing patch is never gated.
Evidence: ARCHITECTURE l.506-507, 686, 923; PROMPT l.46.
Fix: record which machine has which displays in the plan; if the Windows PC lacks them, add an
indirect display driver virtual monitor at 240 and 144 Hz to `ndshell` on Windows.

## Resolution (2026-10-13)

Each finding was checked against the evidence it cites. Section numbers refer to the revised
`docs/plan/ARCHITECTURE.md`; example paths are under `docs/plan/examples/`.

| # | Status | Where |
| --- | --- | --- |
| 1 | Fixed | 3.12: nd owns focus, key contexts and dispatch on every backend, GPUI `Keymap` as matcher only; `NdAppKitPlatform` implements `gpui::Platform` and owns `NSApplication`; first M0 task (16) |
| 2 | Fixed | 3.13 rule 1: queued `Emit`, drain after apply and nested run loops, row cache for data sources, deferred menus/alerts/drags, reentrancy test per widget (M6) |
| 3 | Fixed | 3.13 rule 2: `Synced<T>` with `report`, `cx.programmatic`, text native-owned while focused, `Edited` vs `Replaced`; echo test; `native-widget/src/map_view/appkit.rs`, `notes/src/editor.rs`, `notes/src/model.rs`, new test `opening_a_note_does_not_reorder_the_list` |
| 4 | Fixed | 3.14: identity by explicit id or call site; stateful widgets take ids (`text_editor`, `search_field`, `list`, `source_list` in the examples) |
| 5 | Fixed | 7: `nd-virtual` owns the height tree, AppKit estimation off, O(1) `heightOfRow`, measurement outside callbacks; conformance suite on a real `NSTableView` in tier 2, named in the M5 gate |
| 6 | Fixed | 2.1: maintained fork with rebase gate, full patch table with lane and upstream PR; `nd-os-*` consume patches; AccessKit through GPUI's own integration |
| 7 | Fixed | 3.9: system Swift and SDK pinned by `swift-toolchain.toml`, no SDKROOT or linker from the Nix shell on darwin, `/usr/bin/clang`, `xcrun` SDK, `nd doctor` check |
| 8 | Fixed | 3.9: `links` + `DEP_*` metadata, one aggregate SwiftPM build from the app's `nd::build::app()` (now in every example's `build.rs`), NDKit compiled once, nothing in the source tree, generated registration table; `Gauge.swift` gains `measure`, `Infallible`/`Never` pairing, `Package.swift` takes NDKit from the build |
| 9 | Fixed | 9.5 numeric budgets; M0 baseline, gates in M2, M5, M13; `fast` default decided at M2 by numbers; cached views on the GPUI backend (3.7); production keeps one last frame, full ring only under test (9.2) |
| 10 | Fixed | 16: `NativeSurface` with minimal WKWebView and WebView2 hosts moved to M4; per-example `required-features` from M1; seam spec and `TestBackend` on M0 day one |
| 11 | Fixed | 3.15: `Backend` trait ops, order, transaction, lifetime, threading, live resize, layout boundaries; zig conformance suite ported |
| 12 | Fixed | 5.2 and M0/M2 gates: real edge drag through the OS, every presented frame captured, content edges against the window edge; webview and GPU surface in the window from M4 |
| 13 | Fixed | 9.2, 9.3: macOS bundles record layout and tree diffs plus native view and layer hierarchy, pixels only under `ndshell`; Core Animation frozen and stepped in tier 2 (10.1) |
| 14 | Fixed | 10.2: resident `ndshell agent` in the user session, game guard, optional second session |
| 15 | Fixed | 10.3: machine-wide lock for real input, activation and SCStream; NSEvent-queue tests stay parallel |
| 16 | Fixed | 6.3: framework panels are a top-most in-window view; non-activating `NSPanel` only past the window frame; dismissal on outside click or app deactivation. System `NSMenu` and `NSPopover` stay for menus and anchored popovers, per the OS-structure rule (3.10) |
| 17 | Fixed | 8: Metal blit into our own double-buffered IOSurface pool, set as layer contents |
| 18 | Fixed | 8: Chrome-style CEF on Hyprland runs GPUI's X11 backend under XWayland, as the zig branch did; M15 gates extensions and 1Password on Hyprland; risk row added (17). Flagged for the user |
| 19 | Fixed | 3.2 rewritten to match `notes`; rows take keys by value with nd's own `cx.processor`; `focus(window, cx)` without overlapping borrows; M0 type-checks every example against a stub `nd` crate |
| 20 | Fixed | Palette: notes first, completion only on a prefix match, test types `tri`; `on_submit` notifies; plugin sections under `[plugins.*]` (3.4); overrides colours moved to `overrides/src/theme.rs`, margin replaced by `titlebar()`; `nd check` over `examples/` in CI (15.4) |
| 21 | Fixed | 3.6 and 5.4: `titlebar()` intent with OS-side insets, `window_controls()` resolves to the OS side, lint on px margins beside it |
| 22 | Fixed | 5.4 and 3.10: M2 spike of DWM vs app-drawn caption buttons; app-drawn with `HTMAXBUTTON` is the default unless DWM passes, recorded as an exception |
| 23 | Fixed | 10.1, M3 gate: Fluent goldens only on the Windows host, metric comparison with WinUI Gallery |
| 24 | Fixed | 10.1: tier 2 AppKit mapping test per component, synthetic semantic nodes for menu items and row actions; M6 gate; swipe test uses the synthetic node |
| 25 | Fixed | 6.1, 3.2, `notes/src/note_list.rs`: `list(..).row_actions(..)`; `.swipe_actions` removed |
| 26 | Fixed | 12: `nd check --target web` denies `std::time`/`thread`/`fs`; M11 gate runs tier 1 under `wasm-bindgen-test`; `notes/Cargo.toml` enables jiff's `js` on wasm and the model uses `nd::time::now()` |
| 27 | Fixed | 15.4: `syn`-based checker inside `nd check`, type-aware rules as tier 1 audits |
| 28 | Fixed | 8: engine-by-API matrix with a milestone per cell; WebView2 input forwarding and UIA in the M8 gate |
| 29 | Fixed | 3.11, 11.3, 14, 9.4: zig `ui` components, `config.d`, `clipboard.readImage`, hang behaviour and its replacement |
| 30 | Fixed | 14 and 16: `subsecond` spiked in M1, gated in M10 (edit to visible change under 1.5 s with state kept) |
| 31 | Fixed | 10.5: LLVM `opt-level = 0` + unpacked debuginfo by default; Cranelift optional on Linux with pinned nightly, measured in M0 |
| 32 | Fixed | 7 (g): row output cached by `(key, version)` via `set_items_versioned`, `canvas(..).cache_key`; notes keeps incremental order, search and count indexes (`notes/src/model.rs`) and a self-updating `relative_time` |
| 33 | Fixed | 3.8 and 2.1: one shared renderer and atlas per window; patch routing GPUI `deferred` overlays to nd's overlay layer |
| 34 | Fixed | 5.3: only occluded or minimised windows are throttled |
| 35 | Fixed | 5.3 and M2: gate on a real Windows 240 + 144 Hz pair if present, else an indirect display driver virtual pair from `ndshell`. Needs the user to say which displays the Windows PC drives |

Rejected: none.
