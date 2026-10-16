# react-native-gpui: what to borrow for native feel

Researched 2026-10-15 against [natew/react-native-gpui](https://github.com/natew/react-native-gpui)
at `b225454` (2026-09-26), MIT. Paths below are relative to its `rust/src/`.

It renders React Native components through GPUI on macOS and then fakes the native parts by
parking real AppKit views under or over the transparent Metal view. The JS half (Fabric,
JavaScriptCore, Tamagui) is irrelevant to us: no JS is a non-goal (ARCHITECTURE.md 1). Its GPUI is
a vendored 0.2.2 with the `cocoa`/`objc` crates, so port ideas, never code verbatim.

Our macOS backend is already real AppKit views, so most of their tricks fix a problem we do not
have. What transfers is the set of places where we still paint with a GPU surface inside an
AppKit window, plus the GPUI backend on Linux and Windows.

## Borrow

| # | Their mechanism | Where it lands for us | Value |
| --- | --- | --- | --- |
| 1 | `elements/native_scroll.rs`: a transparent `NSScrollView` sits over each GPU-painted scroll region. AppKit owns wheel routing, momentum, overlay scrollers, rubber band and edge clamping; the clip view's offset is fed back as an absolute scroll position and the GPU side only paints. | Our GPU surfaces in AppKit windows hand-roll `scrollWheel:`: `crates/nd-terminal/src/appkit/mod.rs:315` (scrollback) and `crates/nd-cef/src/mac/view.rs:268` (CEF OSR). Driving both from a hidden `NSScrollView` gives the system scroll feel, the scroller that appears on scroll, and the elastic edge for free. | High |
| 2 | Same principle on Windows: let the OS own touchpad physics. | `crates/nd-gpui/vendor/gpui-fast-windows/src/direct_manipulation.rs` already wraps DirectManipulation, but only the `fast` feature uses it; the default `gpui-pre` path runs momentum through our own nd-gesture router. A patch in `crates/nd-gpui/patches/` that brings the DirectManipulation viewport to `gpui_windows` gives precision touchpads the WinUI scroll curve. | High on Windows |
| 3 | `selection.rs` (model from gpui-component's `TextView`): one window-global selection rect, each selectable text node tests its own glyphs with text-flow semantics, paints its own highlight and registers its substring; Cmd+C joins the registry in reading order. A drag threshold stops a click flashing a one-character selection. | `Label::selectable()` (`crates/nd-components/src/forms/display.rs:104`) is ignored by `nd-backend-gpui` (only source list rows read `node.selectable`, `nav.rs:555`). On AppKit it is per `NSTextField`, so selection never spans two labels on either backend. Port the model into `nd-backend-gpui`; it is the only way a chat transcript or detail pane selects like a document. | High |
| 4 | `drag_out.rs`: drag content out of the app with `NSFilePromiseProvider`, so the drag starts at once and the file is written on a background queue on drop. Test mode prepares the session and logs it instead of calling `beginDraggingSessionWithItems`, so a test run never takes the cursor. | Already a TODO (Phase 7, "file drag with promises"). Their test-mode split is the part to copy into the gate. messages (images) and Files need it. | Medium |
| 5 | `dock.rs`: dock tile badge and attention bounce. In test mode it records the value instead of touching the real dock, readable from the debug dump. | `nd-system` has neither. messages needs the unread badge. Windows equivalent is the taskbar overlay icon plus `FlashWindowEx`; Linux is the Unity launcher API over D-Bus. | Medium |
| 6 | `liquid_glass.rs:74`: seed the window tint after the glass view exists and before the first content frame, so a themed app never flashes raw glass at launch. | Check our launch path for a first frame of untinted `glass()` (macOS) and Mica (Windows). Cheap to verify with the resize gate's frame recorder. | Low, check |
| 7 | Presentation pacing (`plans/presentation-pacing.md`): measure smoothness from `MTLDrawable.addPresentedHandler` / `presentedTime`, coalescing drawables with the same timestamp, since CPU paint time and screen video both lie. | We read `wp_presentation` on Linux (patch 0004). The macOS GPU surfaces (terminal `CAMetalLayer`, CEF IOSurface host) have no present-time gate; add one for M9's display-Hz pacing gate. | Medium |

## Already covered, skip

- Real controls hole-punched through Metal (`native_control.rs`, `hit_passthrough.rs`): our AppKit
  backend is real views end to end, and overlay input leaks are gated (M4).
- Card shadow and corner clip on two separate views (`elements/system.rs`): `nd-backend-appkit/src/glass.rs`
  already splits `masksToBounds` and `shadowPath`.
- AX shadow tree over GPUI (`ax.rs`): not needed while macOS is AppKit. If a GPU-painted region ever
  needs VoiceOver (terminal rows), note their perf fix: skip `setFrame` on AX views whose own and
  parent frame did not move, it was a fixed per-frame cost.
- Invisible on-screen capture window (`liquid_glass.rs:130`, `capture_png.rs`): ndshell's composited
  capture covers it.
- Paint-patch frames (`plans/paint-patch-frames.md`): rewrites recorded paint ops for opacity-only
  ticks in their patched Metal renderer. Interesting for spinners on the GPUI backend, but it depends
  on a damage compositor in their 0.2.2 fork; revisit only if `budget.rs` shows idle animations
  costing whole-window draws.

## Suggested order

1 (terminal first, then CEF), 3, 2 on the Windows lane, then 5 and 4 with the messages port.
Each needs a gate in the existing tiers: for 1, the HID-driven scroll test asserting momentum
continues after the fingers lift and the overlay scroller shows; for 3, a tier 1 test selecting
across two labels and checking the clipboard text.
