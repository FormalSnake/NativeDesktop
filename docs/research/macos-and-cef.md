# macOS backend and embedded browser engines in Rust

Research for the Rust rewrite (request: `docs/plan/PROMPT.md`). Checked 2026-10-13
against crates.io, docs.rs sources, Apple's documentation JSON, upstream GitHub
sources and the old Zig/Swift tree at `~/Developer/NativeDesktop` (branch `zig`,
called "old repo" below). Paths under `NDShell/` mean
`~/Developer/NativeDesktop/swift/Sources/NDShell/`.

## 1. Rust AppKit bindings: objc2 and objc2-app-kit

**Status.** `objc2` 0.6.5 (published 2026-10-05, 132M downloads) and the generated
framework crates `objc2-app-kit` / `objc2-quartz-core` / `objc2-screen-capture-kit`
0.3.2 (published 2025-10-04) are the de facto bindings: winit, wry/tao and GPUI all
depend on them ([crates.io objc2](https://crates.io/crates/objc2),
[objc2-app-kit](https://crates.io/crates/objc2-app-kit)). GPUI's macOS window still
mixes the deprecated `cocoa` crate with `objc2` imports
([gpui_macos/src/window.rs:10-64](https://github.com/zed-industries/zed/blob/main/crates/gpui_macos/src/window.rs)),
so even Zed is mid-migration.

**SDK lag is the real maturity risk.** The released 0.3.2 crates were generated from
the Xcode 26 SDK. `main` now says "generated from the SDKs in Xcode 27.0"
([objc2/src/lib.rs:76](https://github.com/madsmtm/objc2/blob/main/crates/objc2/src/lib.rs))
and was being committed to on 2026-10-12, but there is no release yet. macOS 27 APIs
therefore mean a git dependency, or hand-written `extern_class!`/`msg_send!` shims
for the few new selectors. Example: Apple documents an "interactive glass" property on
`NSGlassEffectView`
([Apple](https://developer.apple.com/documentation/appkit/nsglasseffectview)) that the
0.3.2 binding does not have (it exposes `contentView`, `cornerRadius`, `tintColor`,
`style` only).

**Coverage verified in the 0.3.2 crate source** (`src/generated/*.rs`):

| API | Binding | Notes |
| --- | --- | --- |
| `NSGlassEffectView`, `NSGlassEffectContainerView` (macOS 26) | yes, `NSGlassEffectView.rs` | styles `Regular`, `Clear`; container `spacing` merges nearby glass |
| `NSBackgroundExtensionView` (macOS 26) | yes | extends content under titlebar/sidebar/inspector ([Apple](https://developer.apple.com/documentation/appkit/nsbackgroundextensionview)) |
| `NSToolbar`, `NSToolbarItem`, `NSWindow.setToolbarStyle`, `titlebarSeparatorStyle` | yes, `NSToolbar.rs`, `NSWindow.rs:649,1747` | glass toolbar is automatic for a real NSToolbar on 26 |
| `NSMenu`, `NSStatusItem` | yes, `NSStatusItem.rs` | |
| `NSHapticFeedbackManager` | yes, `NSHapticFeedback.rs` | 3 patterns only (section 4) |
| Force Touch: `NSEvent.pressure/stage/stageTransition/pressureBehavior`, `NSPressureConfiguration`, `pressureChangeWithEvent:` | yes, `NSEvent.rs:801,1033-1048`, `NSResponder.rs:243` | |
| Gestures: `magnification`, `rotation`, `phase`, `momentumPhase`, `trackSwipeEventWithOptions…`, `smartMagnifyWithEvent:`, `swipeWithEvent:` | yes, `NSEvent.rs:836-1057`, `NSResponder.rs:183-238` | |
| `NSView/NSWindow/NSScreen.displayLinkWithTarget:selector:` (macOS 14) | yes but `unsafe`, `NSView.rs:1712` | target/selector API needs a `define_class!` target |

Everything is reachable, mostly `unsafe` and `MainThreadMarker`-gated; subclassing
(custom `NSView`, delegates, `NSTextInputClient`) goes through `define_class!`.

## 2. Hybrid models

### (a) Pure AppKit widgets driven from Rust

What the old repo did (Swift `NDShell` + generated `NDGen`, one NSView per node). In
Rust it is the same design with objc2 instead of Swift. Pros: real controls,
accessibility and text input for free, glass for free. Cons, all observed in the old
repo: a custom box layout had to replace NSStackView
(`NDShell/Layout.swift`, CLAUDE.md "AppKit box layout"); every widget needs a template
per backend; and it cannot meet "all three OSes look the same", because an AppKit
NSTableView is not a GPUI list. It also puts the whole tree through objc message sends
on every mount; there are no published numbers for that cost in Rust, so measure it
before choosing it for content.

### (b) GPUI/Metal content in an AppKit window with native chrome

GPUI renders a window into one `CAMetalLayer` (`set_maximum_drawable_count(3)`,
[gpui_apple/src/metal_renderer.rs:182-183](https://github.com/zed-industries/zed/blob/main/crates/gpui_apple/src/metal_renderer.rs)).
Native chrome lives around it: `NSToolbar` (glass on 26), `NSSplitViewController`
sidebar (floats as glass per Apple's Liquid Glass guidance: "Key navigation elements
like tab bars and sidebars float in this Liquid Glass layer",
[Adopting Liquid Glass](https://developer.apple.com/documentation/technologyoverviews/adopting-liquid-glass)),
traffic lights, `NSMenu` menu bar and context menus, sheets, `NSAlert`, `NSOpenPanel`.

- Cost: chrome is a handful of NSViews created once; content stays on the GPU path.
- Live resize: GPUI already switches the layer to `presentsWithTransaction = true` and
  renders synchronously inside `displayLayer` and on key-window activation
  ([window.rs:3318-3337, 3194-3211](https://github.com/zed-industries/zed/blob/main/crates/gpui_macos/src/window.rs),
  [metal_renderer.rs:525-527](https://github.com/zed-industries/zed/blob/main/crates/gpui_apple/src/metal_renderer.rs)).
  This is what keeps Metal content and native views in the same CA transaction, so a
  sidebar and the content beside it do not wobble apart during a drag-resize. Any
  native view NativeDesktop adds must have its frame set inside that same pass.
- Glass over Metal content: the JaceThings demo ("Finder 1:1 in GPUI using native
  components and liquid glass", 2026-10-05,
  [x.com/JaceThings](https://x.com/JaceThings/status/2107131875944968235)) shows glass
  sampling GPUI content works. The scroll edge effect Apple describes is tied to system
  bars over scroll views ("System bars like toolbars adopt this behavior by default",
  same Apple page); with GPUI scroll content under an NSToolbar we should expect to
  draw the edge fade ourselves. Verify both on day one.
- Sidebar under glass: `NSBackgroundExtensionView` fills under a sidebar by
  extending content at its edges ([Apple](https://developer.apple.com/documentation/appkit/nsbackgroundextensionview)).
  With a single Metal layer as content, the cheap path is one GPUI view spanning the
  whole window behind a transparent sidebar column. The old repo already used
  `NSBackgroundExtensionView` for edge-to-edge content (CLAUDE.md "HIG batch").

### (c) GPUI rendering with AppKit views embedded

Embedded NSViews (WKWebView, CEF windowed, AVPlayerView, MapKit, glass views) are
siblings or children of the Metal-backed view. The binding constraint is **z-order**:
an NSView above the Metal layer covers every GPUI pixel in its rect, including GPUI
popovers, menus and toasts drawn in the same window. And **input**: AppKit hit-testing
runs before GPUI's own hit-test, so a click on a GPUI popover drawn over a WKWebView
goes to the WKWebView. That is the "context menus/floating panels leaking input" bug
from PROMPT.md, structurally.

Fixes, in order of preference:
1. Do not draw overlays in-window over native views. Floating panels, popovers and
   context menus that can overlap an embedded view become separate borderless
   `NSPanel` child windows (`addChildWindow(_:ordered: .above)`), each with its own
   GPUI surface. The old repo already does this for CEF surfaces
   (`NDShell/NDCefSurfaceWindows.swift:174`, `NDCefChromeWindow.swift:720`).
   Native `NSMenu` for context menus sidesteps it entirely (modal tracking loop).
2. Off-screen rendering for web content (section 6) so the page is a GPUI texture and
   z-order is whatever GPUI says.
3. Last resort: a second transparent Metal overlay view above the embedded view.

Latency of (c): the embedded view's own frames (WebKit, Chromium) arrive on their
own timeline; geometry must be committed in the same CA transaction as GPUI's
synchronous present or the view trails by a frame during resize. The old repo hit the
extreme case with CEF under a sliding sidebar and replaced per-frame resize with a
stretched still (`NDShell/NDCefMotion.swift:1-30`).

### What "same on all three OSes, native patterns" implies

Content (lists, cards, editors, settings rows) is GPUI everywhere, so it looks the
same. Chrome is per-platform: AppKit toolbar/sidebar glass/traffic lights/NSMenu on
macOS; GPUI-drawn client-side decorations on Linux (none on Hyprland); WinUI or GPUI
title bar on Windows. The app tree declares intent (`Toolbar`, `Sidebar`,
`WindowControls`, `ContextMenu`) and each backend maps it, which is the old repo's
"fix UI at the framework level" rule carried forward.

## 3. Traffic lights, drag regions, double-click

**By coordinates.** GPUI's `traffic_light_position` captures the stock frames once,
then moves the three `standardWindowButton`s and the container, re-applying on
resize, key changes and full-screen transitions, and restoring the stock frames
captured before full screen
([window.rs:687-763](https://github.com/zed-industries/zed/blob/main/crates/gpui_macos/src/window.rs)).
Electron's `setWindowButtonPosition` does the same through
`close.superview.superview` and only works with a hidden or custom titlebar
([Electron #37094](https://ayakael.net/mirrors/electron/commit/0de1012280eb49dc1c63f34ba3548b36df82ca9c),
[#21781](https://ayakael.net/mirrors/electron/commit/578185070687f65654ad175603b633712a9ed129)).
AppKit re-lays out the titlebar on every resize, so any one-shot move reverts
([Xojo forum](https://forum.xojo.com/t/how-to-reposition-window-traffic-lights/20844)).

**By embedding (Arc-style).** No public write-up of Arc's implementation exists (none
found). The old repo already ships the equivalent: `<windowcontrols>` is a slot view
in the app tree; the window's real buttons (keeping their hover group, menu and
accessibility) are moved onto the slot's rect, re-asserted on titlebar relayout, and
"ride" with the slot during a sidebar slide via an additive layer animation, with
ghost stand-ins because the titlebar will not keep a button outside its bounds
(`NDShell/WindowControls.swift:1-60`). Port that design: an app element owns a
`WindowControls` placeholder; the backend moves the real buttons to it each layout
pass. Do not create fresh buttons with `standardWindowButton(_:for:)` in a custom
view: the three-button hover glyph group depends on private titlebar behaviour
(Electron needed extra machinery for `customButtonsOnHover`,
[#26789](https://ayakael.net/mirrors/electron/commit/e01b1831d96d5d68f54af879b00c617358df5372)).

**Full screen.** The titlebar moves into a separate full-screen toolbar window that
reveals on menu-bar hover; positions must be restored to stock before entering and
re-applied after exit (GPUI keeps `pre_fullscreen_traffic_light_frames`,
window.rs:689).

**Drag regions.** Call `NSWindow.performDrag(with:)` (macOS 10.11,
[Apple](https://developer.apple.com/documentation/appkit/nswindow/performdrag(with:)))
from mouse-down inside a framework `DragRegion`; it gives native snapping, tiling and
Spaces behaviour. Interactive children inside the region opt out by hit-test.

**Double-click.** Read `AppleActionOnDoubleClick` from the global domain and zoom,
minimize or do nothing; GPUI does exactly this
([window.rs:2153-2169](https://github.com/zed-industries/zed/blob/main/crates/gpui_macos/src/window.rs)).

## 4. Gestures and haptics

- **Pinch / rotate / smart zoom / force click.** `magnifyWithEvent:`,
  `rotateWithEvent:`, `smartMagnifyWithEvent:`, `quickLookWithEvent:`,
  `pressureChangeWithEvent:` (bindings above). GPUI forwards pinch with phase and
  pressure with stage
  ([gpui_macos/src/events.rs:191-255](https://github.com/zed-industries/zed/blob/main/crates/gpui_macos/src/events.rs)),
  but has no rotate or smart-magnify event.
- **Momentum.** GPUI maps only `phase()` for scroll and never reads
  `momentumPhase()` ([events.rs:259-285](https://github.com/zed-industries/zed/blob/main/crates/gpui_macos/src/events.rs)),
  so it cannot tell finger scrolling from inertia. Rubber banding and
  overscroll-to-refresh need that split (begin elastic on finger, spring back when
  momentum ends). NativeDesktop's scroll primitive must carry both phases and own the
  spring itself; GPUI does not provide one.
- **Page swipes.** Two-finger horizontal swipes come in as scroll events; the app
  calls `trackSwipeEventWithOptions:dampenAmountThresholdMin:max:usingHandler:` when
  `NSEvent.isSwipeTrackingFromScrollEventsEnabled` is true
  ([Apple](https://developer.apple.com/documentation/appkit/nsevent/isswipetrackingfromscrolleventsenabled),
  how Gecko does it: [Tor/Gecko commit](https://gitlab.torproject.org/clairehurst/tor-browser/-/commit/1cbc5fe496844449b1a631cb46e3302e8ab38053)).
  Three-finger swipes arrive as `swipeWithEvent:` only when the user's trackpad
  setting assigns three fingers to "swipe between pages" (GPUI hooks it,
  window.rs:209); system gestures (Mission Control, Spaces) are never delivered.
- **Haptics.** `NSHapticFeedbackManager` is Force-Touch-trackpad only and offers
  three patterns (`Generic`, `Alignment`, `LevelChange`) with timing `Default`, `Now`,
  `DrawCompleted` (`objc2-app-kit NSHapticFeedback.rs:16-58`,
  [Apple](https://developer.apple.com/documentation/appkit/nshapticfeedbackmanager)).
  No public custom waveform API. Map framework semantics (snap, detent, threshold
  crossed) onto those three, and fire `Alignment` at `DrawCompleted` so the tick lands
  on the frame that shows the snap.
- **Synthesising gestures for tests.** No public CGEvent constructor; the old repo
  posts the WindowServer layout directly (type 29, subtype 8 = zoom, field 113 =
  magnification, field 132 = phase) at the HID tap (`NDShell/InputHelper.swift:203-221`).
  Private and undocumented, but it is what makes pinch testable.

## 5. Multi-monitor ProMotion

- `CVDisplayLink` is deprecated as of macOS 15 with the message "use
  NSView.displayLink(target:selector:), NSWindow.displayLink(target:selector:), or
  NSScreen.displayLink(target:selector:)"
  ([Apple JSON](https://developer.apple.com/documentation/corevideo/cvdisplaylinkcreatewithactivecgdisplays(_:))).
- `NSView.displayLink(target:selector:)` (macOS 14) "will be invoked in-sync with the
  display the view is on. If the view is hidden, or not on any display, the callback
  will not be invoked"
  ([Apple](https://developer.apple.com/documentation/appkit/nsview/displaylink(target:selector:))).
  That gives per-window, per-screen pacing and automatic handoff when a window moves
  from the 120 Hz built-in panel to a 60 Hz external, with no screen-change plumbing.
  `preferredFrameRateRange` is a best-effort hint
  ([Apple](https://developer.apple.com/documentation/quartzcore/cametaldisplaylink/preferredframeraterange)).
- GPUI still uses `CVDisplayLink`: one immortal link per display in a static registry
  (teardown races caused segfaults, documented at
  [display_link.rs:1-50](https://github.com/zed-industries/zed/blob/main/crates/gpui_macos/src/display_link.rs)),
  restarted from `windowDidChangeScreen:` (window.rs:3121-3124). It works, but it
  fights a deprecated API; the `NSView` link removes the whole registry.
- `NSScreen.maximumFramesPerSecond` (macOS 12) gives the per-screen cap for the
  profiler HUD and for choosing animation step sizes
  ([Apple](https://developer.apple.com/documentation/appkit/nsscreen/maximumframespersecond)).

## 6. Browser engines

### CEF in Rust: `cef` crate (tauri-apps/cef-rs)

- `cef` 154.5.0+154.0.34 (2026-10-07, CEF/Chromium 154, 287k downloads); repo pushed
  2026-10-13, 478 stars ([crates.io](https://crates.io/crates/cef),
  [GitHub](https://github.com/tauri-apps/cef-rs)). Full C API binding, all three OSes
  on x86_64 and arm64, a `bundle-cef-app` tool that builds the macOS helper bundles,
  `NIX_CEF_BINARY=1` to use nixpkgs' `cef-binary`
  ([README](https://github.com/tauri-apps/cef-rs/blob/HEAD/README.md)). Tracks CEF
  releases closely, so pinning matters.
- **Zero-copy OSR is built in**: feature `accelerated_osr`, module
  `cef/src/osr_texture_import/{iosurface,d3d11,dmabuf}.rs`, and an `examples/osr` that
  imports the shared texture into wgpu in `on_accelerated_paint`
  ([examples/osr](https://github.com/tauri-apps/cef-rs/tree/HEAD/examples/osr)).
  Alternatives (raw `cef-dll-sys`, CefSharp-style wrappers) offer nothing better.

### Windowed vs off-screen

| | Windowed (Alloy, `parent_view`) | Off-screen (`windowless_rendering_enabled`) |
| --- | --- | --- |
| Pixels | Chromium NSView inside our window (not a child window) | IOSurface/D3D11/dmabuf per frame into our renderer |
| Z-order with GPUI overlays | web view covers GPUI pixels; overlays need child panels (section 2c) | perfect: page is a GPUI texture |
| Resize | autoresizing subview, lockstep if frame set in GPUI's transaction | lockstep by construction, but Chromium paints the new size a few frames later |
| IME, a11y, select popups, drag | free | all hand-wired (below) |
| Linux Wayland | not shipped upstream (CEF #2804; ANGLE Wayland merged 2026-05, [Phoronix](https://www.phoronix.com/news/ANGLE-Merges-Wayland)) | works via dmabuf |

Facts from the CEF headers
([cef_types_mac.h](https://github.com/chromiumembedded/cef/blob/master/include/internal/cef_types_mac.h),
[cef_render_handler.h](https://github.com/chromiumembedded/cef/blob/master/include/cef_render_handler.h),
[cef_types.h](https://github.com/chromiumembedded/cef/blob/master/include/internal/cef_types.h)):
- "Alloy style will always be used if |windowless_rendering_enabled| is true or if
  |parent_view| is provided": no Chrome-style features (extension runtime) in an
  embedded browser. The old repo works around this by lifting the web contents NSView
  out of a hidden CEF Views window (`NDShell/NDCefChromeWindow.swift:1-27`).
- OSR handle on macOS is an IOSurface; "The handle's resource cannot be cached ...
  the contents should be copied to a texture owned by the client application" (one
  GPU blit per frame, fine).
- `windowless_frame_rate` defaults to **30**; set it to the display's Hz per window
  and change it with `SetWindowlessFrameRate` on screen change, or use
  `external_begin_frame_enabled` to drive Chromium's BeginFrame from our display link.
- OSR must implement: `GetScreenInfo`/`GetRootScreenRect` (scale, screen),
  `OnPopupShow` + `PET_POPUP` paints (`<select>`), `StartDragging`,
  `OnImeCompositionRangeChanged` (candidate window placement, with
  `ImeSetComposition`/`ImeCommitText` from our `NSTextInputClient`),
  `OnTextSelectionChanged`. Context menus go through `CefContextMenuHandler::RunContextMenu`
  so we show a native `NSMenu`/GPUI menu ourselves. Accessibility for OSR pages is not
  native on macOS and has to be bridged from CEF's accessibility events; budget for it.

**How the old repo made CEF feel part of the window:** Alloy windowed browser created
only once the view is in a window, because an unparented create opens a top-level
Chromium window over the app (`NDShell/NDCefWebView.swift:13-17`);
`external_message_pump = 0`, OSR off (`NDCefEngine.swift:100-101`); Chromium's own
floating surfaces adopted as child windows pinned over the view and hidden with the
tab (`NDCefSurfaceWindows.swift:150-185`); a still image instead of per-frame resize
during sidebar slides (`NDCefMotion.swift`). That is 10k+ lines of Swift
(`NDCef*.swift`) to port.

### WKWebView via objc2 / wry

- `wry` 0.57.0 (2026-09-08) builds on `objc2-web-kit`; as a child it does
  `ns_view.addSubview(&webview)` with autoresizing masks
  ([wry src/wkwebview/mod.rs:500-525, 668-684](https://docs.rs/crate/wry/0.57.0/source/src/wkwebview/mod.rs)),
  i.e. an in-window subview, no floating window. Zero bytes shipped.
- WKWebView has no public frame-callback API, so it can only be embedded as an
  NSView; z-order rules of section 2c always apply. wry assumes it owns the event
  loop integration via tao/winit; for NativeDesktop use `objc2-web-kit` directly and
  keep wry as reference code. The old repo's `NDWebView.swift` (1,433 lines) is the
  feature checklist.

### Packaging and helpers

- macOS: `Chromium Embedded Framework.framework` is 251 MB unpacked (measured on an
  Electrobun build of CEF 125 on this machine, 60 MB of it `Resources/`), plus five
  helper apps (`Helper`, `Alerts`, `GPU`, `Plugin`, `Renderer`) under
  `Contents/Frameworks`, signed inside-out with JIT entitlements on GPU and Renderer
  (old repo `docs/webview.md:1150-1153`).
- Linux: `libcef.so` is 1.43 GB unstripped, about 269 MB after `strip`; trim
  `locales/`; `chrome-sandbox` needs root 4755 or the userns fallback
  (`docs/webview.md:1144-1150`).
- Windows: helpers are the same executable re-launched with `--type=`; cef-rs builds a
  DLL plus `bootstrap.exe` when the `sandbox` feature is on (cef-rs README).
- Keep the old rule: CEF loaded at runtime, packaging ships zero Chromium bytes unless
  the app config enables it (`docs/webview.md:1135-1143`).

## 7. macOS automation for the test tool

- **Input without stealing the user's cursor:** post `NSEvent`s into the app's own
  queue (`NSApp.postEvent`/`sendEvent`), as the old M16 harness did; drags must post
  the whole down/dragged/up batch in one marshal because AppKit tracking loops block
  the main thread (old CLAUDE.md "Agentic testing"). No TCC needed, works with the
  window in the background, runs in parallel with the user's work.
- **HID-level fidelity when needed:** `CGEventPost(kCGHIDEventTap)` from a helper
  (`--nd-input`, `NDShell/InputHelper.swift`) needs Accessibility and PostEvent
  grants and moves the real cursor; keep it opt-in.
- **Frame recording at display refresh:** two paths.
  1. In-process, preferred: we own the Metal renderer, so copy each presented
     drawable to a ring of IOSurfaces and stamp it with `MTLDrawable.presentedTime`
     (macOS 10.15.4, "the host time ... when the drawable was displayed onscreen",
     [Apple](https://developer.apple.com/documentation/metal/mtldrawable/presentedtime)).
     Exact frames, exact timing, dropped-frame detection, no TCC. Misses native chrome
     and WebKit/CEF windowed pixels.
  2. Composited: `SCStream` with `minimumFrameInterval = 1/Hz`
     ([Apple](https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/minimumframeinterval));
     Apple's samples use 60 fps, and a 2026 forum report says delivery stays at 60 on
     iPad even at 1/120 ([thread 848296](https://developer.apple.com/forums/thread/848296)).
     Unverified at 120 Hz on macOS; test it before promising ProMotion recordings.
     Old repo only takes stills (`SCScreenshotManager`, `NDShell/AutomationCapture.swift:46-54`).
- **TCC with SIP off:** the old `--nd-grant` writes `kTCCServiceScreenCapture`,
  `kTCCServiceAccessibility`, `kTCCServicePostEvent` rows into
  `/Library/Application Support/com.apple.TCC/TCC.db` via `sudo sqlite3`, with a csreq
  of `identifier "<id>"` so the grant survives rebuilds, keyed by the real path
  because SwiftPM's `.build/release` is a symlink (`NDShell/CaptureGrant.swift:13-91`).
  Same approach in Rust; cargo's `target/` must be on a stable real path (the external
  drive per kickoff decisions). The capture runs in a re-spawned helper with
  responsibility disclaimed so TCC checks the host, not the terminal (old CLAUDE.md).
  Also suppress the Sequoia-era periodic re-approval prompt
  (`ScreenCaptureApprovals.plist` / replayd, [TidBITS](https://tidbits.com/2024/09/23/how-to-avoid-sequoias-repetitive-screen-recording-permissions-prompts/),
  [tinyapps](https://tinyapps.org/blog/202409180700_disable_sequoia_nag.html)); check it
  still applies on 27.
- **No VM, no collisions:** a private `CGVirtualDisplay` gives tests their own
  off-to-the-side display at a chosen refresh rate (120 Hz to test ProMotion pacing
  on any Mac). Chromium uses it in its own tests
  ([virtual_display_util_mac.mm](https://gitcode.com/openharmony-tpc/chromium_src/blob/master/ui/display/mac/test/virtual_display_util_mac.mm));
  private and can change in any release
  ([go-macos/virtualdisplay](https://pkg.go.dev/github.com/go-macos/virtualdisplay)).
  Parallel test apps then need only unique socket paths and data dirs, not VMs.

## Verdict for NativeDesktop

**Recommendations**
1. macOS backend = model (b): GPUI-style Metal content, AppKit chrome (NSToolbar,
   NSSplitViewController glass sidebar, NSMenu, sheets, alerts, panels, status item).
   Model (a) only for leaf controls that must be native (text field with system
   spell/autofill is not one; GPUI handles text). Model (c) only for engines and media.
2. One `nd-appkit` crate owns all `unsafe` objc2 code. Depend on objc2 `main` (Xcode 27
   SDK) via a pinned git rev until the next release; plan a shim file for any macOS 27
   selector missing upstream.
3. Pacing: `NSView.displayLink` per window (macOS 14+ floor), not CVDisplayLink. Expose
   the current screen Hz to the profiler.
4. Overlays (popovers, menus, toasts, command palette) are child `NSPanel`s whenever an
   embedded native view exists in the window; native `NSMenu` for context menus. This
   kills the input-leak class by construction. Add a gate that clicks a popover drawn
   over a webview and asserts the webview saw nothing.
5. `WindowControls` slot + `DragRegion` are framework primitives; port the old
   `WindowControls.swift` ride/ghost logic and GPUI's `AppleActionOnDoubleClick`
   handling. Coordinates mode is the slot pinned to a fixed rect.
6. Gesture layer owns phases: finger vs momentum, rubber band spring, two-finger page
   swipe via `trackSwipeEvent`, pinch/rotate/smart zoom/force click, haptics mapped to
   the three system patterns.
7. Web: WKWebView (objc2-web-kit, windowed subview) as the default engine. CEF via the
   `cef` crate in **OSR with `accelerated_osr`** as the opt-in engine: one code path on
   all three OSes, solves Wayland, solves z-order. Drive it with
   `external_begin_frame_enabled` from the display link.
8. Test tool: in-process NSEvent input + in-process drawable recording by default;
   SCStream and HID CGEvents behind `--composited`; `--nd-grant` ported as is; a
   virtual display for parallel, VM-free runs.

**Risks**
- objc2 releases trail Xcode by months; macOS 27 APIs need git deps or shims.
- CEF OSR moves IME, `<select>`, drag-and-drop, accessibility and context menus onto
  us. The old windowed integration got these free and still took 10k lines. Budget
  OSR as its own milestone with gates per item; accessibility is the hardest.
- CEF Chrome-style features (extensions) are unavailable to embedded browsers; the old
  lift-out hack is fragile and windowed-only.
- Glass sampling Metal content and scroll edge effects under GPUI are shown by one demo
  video, not documented; verify in week one.
- Private APIs in the test tool (gesture event fields, CGVirtualDisplay, TCC.db writes,
  responsibility disclaim) can break on any macOS update; keep them in the test
  helper, never in shipped app code.
- 120 Hz ScreenCaptureKit delivery is unconfirmed on macOS.
- CEF adds ~250 MB (macOS) / ~270 MB stripped (Linux) per app; never default it on.
