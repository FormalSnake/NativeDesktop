# GPUI as the NativeDesktop rendering core

Researched 2026-10-13. Source of truth for code claims is zed-industries/zed at
`60b0983f` (main, 2026-10-13T12:44Z). Links below use the prefix
`Z = https://github.com/zed-industries/zed/blob/60b0983f189fb183ba3c95f44e0a7ae6aba8c640/`.
Crate facts come from the crates.io API on the same day.

## 1. Platform backends and standalone use

Upstream GPUI is now split into a core crate plus one crate per platform
(`Z/crates/`): `gpui`, `gpui_platform` (picks the backend), `gpui_macos` +
`gpui_apple` (Metal, CoreText), `gpui_windows` (DirectX 11, DirectWrite),
`gpui_linux` (Wayland + X11), `gpui_wgpu` (shared wgpu renderer, cosmic-text),
`gpui_web` (wasm).

| Backend | Renderer | Maturity (2026-10) | Evidence |
| --- | --- | --- | --- |
| macOS | Metal | Most mature, Zed's primary platform since 2023 | `Z/crates/gpui_apple/src/metal_renderer.rs` |
| Windows | DirectX 11 + DirectWrite | Stable since Zed for Windows, 2025-10-15 | [zed.dev blog](https://zed.dev/blog/zed-for-windows-is-here) |
| Linux Wayland/X11 | wgpu (Vulkan, GLES fallback) | Usable, still churning. Blade was removed for wgpu in PR #46758 (2026-02-13); resize panics and device-lost fixes followed in March | [commit af8ea0d](https://git.secluded.site/zed/commit/af8ea0d6c26192c45f44f473c0d4a7d6f72ed018), [releases](https://zed.dev/releases/stable?page=6) |
| Web | wgpu WebGPU, WebGL2 fallback | Experimental: one canvas, one top-level window, no reopen, no file prompts, no menus | `Z/crates/gpui_web/src/gpui_web.rs:1-4`, `platform.rs:113,479` |

Linux feature flags: `gpui_linux` defaults to `["wayland","x11"]` and both pull
`accesskit_unix` and `gpui_wgpu` (`Z/crates/gpui_linux/Cargo.toml:16-47`).
`gpui_wgpu` on Linux enables `vulkan` and `gles` (`Z/crates/gpui_wgpu/Cargo.toml:45`).
The GL path costs about 59 MB RSS even when Vulkan wins; the user's own
youtubemusic patches that out (`~/Developer/youtubemusic/Cargo.toml`, patch
`gpui-pre-wgpu`) mirroring upstream PR #63313, which is still OPEN.

**Standalone use, three channels:**

- `gpui` on crates.io is stale: max 0.2.2 from 2025-10-22, owned by
  `zed-industries`. The README still says `gpui = "*"` plus `gpui_platform`
  (`Z/crates/gpui/README.md`), which no published version satisfies.
- `gpui-pre` (0.3.0 on 2026-09-03 to 0.3.9 on 2026-10-12): weekly snapshots of
  zed main, one crate per Zed crate (`gpui-pre-platform`, `-macos`, `-linux`,
  `-windows`, `-web`, `-wgpu` ...). Published by `huacnlee` (Longbridge), not by
  Zed. This is what the whole third-party ecosystem pins today.
- `gpui-fast` 0.1.x ([longbridge/gpui-fast](https://github.com/longbridge/gpui-fast),
  created 2026-09-24, 204 stars): a performance fork with retained mode, scroll
  layers and window composition (see sections 6 and 8).
- Git dependency on zed main also works, but Zed has no stability promise and
  pre-1.0 breaks land weekly (`Z/crates/gpui/README.md`, "There will often be
  breaking changes").

The user's two GPUI apps (`messages`, `youtubemusic`) already depend on
`gpui-kit = "0.6.6"`, which pins `gpui-pre` (`~/Developer/messages/Cargo.toml`).

## 2. Multi-monitor and variable refresh

**macOS.** Frame pacing is one immortal `CVDisplayLink` per display in a static
registry; windows subscribe (`Z/crates/gpui_macos/src/display_link.rs:1-51`).
On `windowDidChangeScreen:` the window restarts its link against the new
display, updates the scale factor and reports the display change
(`Z/crates/gpui_macos/src/window.rs:3121-3128`). CVDisplayLink follows the
display's own refresh, so a ProMotion panel paces at up to 120 Hz and an
external 60 Hz monitor at 60. There is no `CADisplayLink`/`preferredFrameRateRange`
request, so GPUI cannot ask ProMotion for a specific rate (say 120 only during
animation); that is a gap if we want ProMotion power behaviour to match AppKit.

**Throttling (all platforms).** `gpui/src/window.rs:1813-1857`: a frame with no
pending work is skipped (`FrameSkipReason::NoRenderNeeded`), an inactive window
is capped at about 30 fps unless input is high-rate, and thermal
Serious/Critical caps at 60 fps (`gpui_apple/src/thermal.rs`).

**Wayland.** Paced by per-surface `wl_surface.frame` callbacks
(`gpui_linux/src/linux/wayland/window.rs:938-1029`), so each window runs at the
rate of the output the compositor presents it on. Fractional scale and
viewporter are bound (`window.rs:21-28`). Known bug class: GNOME/Mutter with
mixed refresh (165 + 60 Hz) went janky in fullscreen
([zed#51397](https://github.com/zed-industries/zed/issues/51397), closed stale,
not fixed), and GNOME 50 VRR sluggishness
([zed#53522](https://github.com/zed-industries/zed/issues/53522), open;
workaround XWayland). Hyprland is a different compositor, so neither proves a
g815 problem, but both say: test 240 + 144 Hz on g815 early.

**X11.** One `refresh_rate` per window from randr (`x11/client.rs:116`), so a
window dragged to the other monitor uses the stale rate until re-queried.

**Windows.** One `VSyncProvider` built on `DwmFlush` with the DWM interval,
default 16.6 ms (`gpui_windows/src/vsync.rs:24-40`). That paces to the DWM
composition clock, not per-monitor. On g815 (240 + 144 Hz) verify whether a
window on the 144 Hz screen runs at 240 or 144.

## 3. Accessibility, IME, text, gestures, haptics

- **Accessibility: AccessKit, in upstream, on all three desktops.**
  `gpui/src/window/a11y.rs:1-40` (architecture), user guide
  `gpui/src/_accessibility.rs`. Adapters: `accesskit_macos`
  (`gpui_macos/src/window.rs:712,2376`), `accesskit_windows`
  (`gpui_windows/Cargo.toml:24`), `accesskit_unix` over D-Bus (Linux).
  Nodes come from elements with ids; still active work (PR #60397 landmarks and
  menus, 2026-07-13; #61926 identifiers, 2026-08-07; #64143 adapter leak,
  2026-09-20). gpui-kit layers roles/labels/states on top and tests them.
- **IME.** macOS implements `NSTextInputClient` (`insertText:replacementRange:`,
  `setMarkedText:...`, `gpui_macos/src/window.rs:259-263`). Wayland uses
  `zwp_text_input_v3` with cursor rectangles (`wayland/client.rs:54-191`). X11
  uses XIM (`x11/xim_handler.rs`). Web mirrors a hidden input
  (`gpui_web/src/ime_mirror.rs`). Zed ships CJK input on all of these, so it is
  production grade on desktop.
- **Text.** CoreText on macOS, DirectWrite on Windows, cosmic-text + swash on
  Linux (`gpui_wgpu/src/cosmic_text_system.rs`). Bidi caret handling is weak
  ([zed#20545](https://github.com/zed-industries/zed/issues/20545) open); the
  third-party `gpui-bidi` crate exists. Linux colour emoji needs a workaround
  (messages `CLAUDE.md`, "GPUI rules that bit us": cosmic-text only paints
  colour for a face named `NotoColorEmoji`).
- **Trackpad.** macOS receives `magnifyWithEvent:`, `swipeWithEvent:`,
  `pressureChangeWithEvent:` and `scrollWheel:` with phases
  (`gpui_macos/src/window.rs:180-212`); force click surfaces as
  `MousePressureEvent`/`PressureStage` (`gpui/src/interactive.rs:228-250`).
  Wayland binds `zwp_pointer_gesture_pinch_v1` only, no swipe or hold
  (`wayland/client.rs:44-45`). Windows uses DirectManipulation for precision
  touchpads (`gpui_windows/src/direct_manipulation.rs`). Rotation is not
  exposed anywhere. `gpui/src/gestures.rs` adds a touch recognizer arena (tap,
  pan, long press, momentum) but says "Pinch recognition is not implemented
  yet" (`gestures.rs:488`); it targets mobile touch, not trackpads.
- **Momentum scroll.** macOS gets OS momentum phases for free. Linux Wayland
  has no kinetic scrolling ([zed#14473](https://github.com/zed-industries/zed/issues/14473),
  open since 2024-07); gpui-base ships `scroll_bounce.rs`, not inertia.
- **Haptics.** None in GPUI. No `NSHapticFeedbackManager` call anywhere in
  `gpui_macos`. We would add it ourselves (a few lines of objc2), mac only.

## 4. Virtualization

- `uniform_list`: measures the first item, lays out the rest arithmetically,
  `scroll_to_item` with Top/Center/Bottom/Nearest strategies,
  `is_scrolled_to_end`, `scroll_to_bottom` (`gpui/src/elements/uniform_list.rs:1-253`).
- `list` + `ListState`: variable height, sum-tree of measured/unmeasured items,
  `ListAlignment::Bottom`, overdraw, `splice`/`reset`, `measure_all`,
  `with_uniform_item_height` size hints, remeasure that preserves pixel or
  fractional offset of the scroll-top item, and `FollowMode::Tail` for
  stick-to-bottom (`gpui/src/elements/list.rs:1-400`).
- Caveat stated in the module doc: items outside the viewport must not change
  height unless you call `splice`/`reset` (`list.rs:1-4`). That contract is
  where "last rows disappearing" bugs come from.
- Recent fixes show the area is maturing but was buggy this year: follow-tail
  refactor to fix snapping (PR #53101, 2026-04), follow-tail not re-engaging
  after scrollbar drag (#54570), bottom-aligned scrollbar thumb vanishing at the
  end (#51198), and stale heights making list items paint over each other until
  taffy 0.13 (#62498, closed 2026-08).
- gpui-kit adds `virtual_list.rs` (base and component), `table.rs` for
  hundreds of thousands of rows, and `message_scroller.rs`, "a virtualized
  message list with optional scrollbar and jump-to-latest UI".
- The messages app already uses `ListState` + `ListAlignment::Bottom` +
  `FollowMode::Tail` (`~/Developer/messages/crates/desktop/src/thread.rs:128-166`)
  and `uniform_list` for the sidebar (`sidebar.rs:459`).

## 5. Visual effects and window chrome

- `WindowBackgroundAppearance` = `Opaque | Transparent | Blurred |
  MicaBackdrop | MicaAltBackdrop` (`gpui/src/platform.rs:2766-2785`).
  macOS `Blurred` inserts an `NSVisualEffectView` subclass under the Metal view
  (`gpui_macos/src/window.rs:300-308,1864-1899`). Wayland blur only through
  `org_kde_kwin_blur` (KDE) (`wayland/window.rs:31,111`). Hyprland does not
  speak that protocol; it blurs windows with alpha by its own `decoration:blur`
  rules, so `Transparent` plus a Hyprland rule is the path there (verify on g815).
- **Liquid Glass.** Not upstream. Open PR #58833 "glass content rendering"
  (huacnlee, 2026-06-08) adds an alpha-preserving quad pipeline so content over
  a glass window does not punch holes. PR #54512 (BlurRect/LensRect shader
  primitives) was closed unmerged. The working route today is what
  [gpui-macos](https://github.com/Rhein-Industries/gpui-macos) (created
  2026-10-10, 1 star) does: grab the `NSView` from `raw_window_handle` and
  insert a real `NSGlassEffectView` underlay below the transparent GPUI layer
  (`src/native/macos.rs:101-199`), with a separate `NSPanel` for glass popups
  (`:354-401`). Same idea also gives it an AppKit `NSToolbar`.
- **Titlebar and controls.** `TitlebarOptions { appears_transparent,
  traffic_light_position }` plus a runtime `set_traffic_light_position`
  (`gpui/src/platform.rs:1298`; mac impl `window.rs:718-775,1626`). It
  re-applies after document-edited and fullscreen changes (`window.rs:1918-1932`).
  Moving traffic lights by coordinates is therefore built in; "embedding" them
  in a moving element means calling the setter from layout each frame.
- **Drag regions.** `start_window_move` (`performWindowDragWithEvent:` on mac,
  `window.rs:2220-2230`), `start_window_resize(edge)`, `show_window_menu`,
  and `WindowControlArea` hit testing for Windows caption snap layouts
  (`platform.rs:1272-1347`). The messages app hit two bugs: cached views drop
  their window control areas, and the Windows caption hit test sees every
  hitbox so title-row buttons must `occlude()` (messages `CLAUDE.md`).
- **Wayland CSD.** `WindowDecorations::{Server, Client}` with
  `request_decorations`, `window_decorations`, `window_controls` and
  `set_client_inset` for shadow insets (`platform.rs:783-795,1323-1347`). GPUI
  asks for `zxdg_toplevel_decoration_v1`; Hyprland grants server decorations,
  which on Hyprland means no titlebar, so apps draw no controls. gpui-kit's
  `TitleBar` + `window_border.rs` draw controls only when client-decorated.
- **Popups and layer shell.** `WindowKind::PopUp` and a parent-anchored native
  popup kind for menus/tooltips (`platform.rs:2706-2720`,
  `gpui/src/platform/popup.rs`), Wayland `xdg_popup` (`wayland/popup.rs`) and
  layer-shell (`wayland/layer_shell.rs`). Native popups are the fix for menus
  that need to escape the window and not leak input.

## 6. Native views inside a GPUI window

- **Jace's post** (2026-10-05, [x.com/JaceThings](https://x.com/JaceThings/status/2107131875944968235),
  read via fxtwitter API): "hello, Finder 1:1 in GPUI using native components
  and liquid glass", a video only. No public repo (his GitHub has none).
  Technique is most likely the gpui-macos one above.
- **Upstream mechanism:** none beyond `raw_window_handle`
  (`gpui_macos/src/window.rs:2423`). A native subview added over the Metal view
  covers every GPUI overlay in that rect: menus, popovers and tooltips go
  behind the webview.
- **The real fix is window composition**, upstream PR
  [#62379](https://github.com/zed-industries/zed/pull/62379) (huacnlee,
  2026-08-09, OPEN). It splits presentation into ordered surfaces: GPUI base,
  native surface (WKWebView/WebView2/subsurface), GPUI overlay on top, with
  "surface ordering, geometry, compositor recovery, focus, and input handling"
  on macOS, Windows, Wayland and X11. It ships today in gpui-fast:
  macOS overlay layer, Windows WebView2 composition, Wayland subsurfaces, X11
  child window plus an overlay window blended by the compositor
  ([gpui-fast README](https://github.com/longbridge/gpui-fast)).
- **gpui-webview** (formerly `gpui-wry`, 0.7.3, wry based,
  `longbridge/gpui-kit/crates/webview/README.md`): self-described
  experimental. GPUI overlays above the page work on macOS and X11; on Windows
  the WebView2 still covers overlays; **Wayland is not supported** (WebKitGTK
  needs X11/XWayland). It requires the gpui-fast backend.
- Other: `bezel-browser` 0.2.14 ("a platform webview hosted in a gpui window"),
  no CEF integration crate exists for GPUI.
- **Resize sync** is manual: `WebView::set_bounds()`/`set_visible()` each
  frame. Native views resize one frame late unless the composition tree drives
  geometry inside the GPUI frame (what #62379 does).
- For CEF, off-screen rendering into a GPUI texture (`surface.rs` element) is
  the route that guarantees z-order and input routing on every platform,
  at the cost of OSR performance. The composition route keeps CEF native
  speed but needs per-platform work we would own.

## 7. Testing hooks

- `TestAppContext` over `TestPlatform`: deterministic dispatcher, seeds,
  `advance_clock`, simulated keystrokes and mouse (`gpui/src/app/test_context.rs`,
  `platform/test/`).
- `HeadlessAppContext`: cross-platform, real text shaping, optional GPU
  renderer factory and `capture_screenshot` (`gpui/src/app/headless_app_context.rs:1-30`).
- `VisualTestAppContext` (macOS only): real Metal rendering in windows placed
  at -10000,-10000, `simulate_keystrokes/input/mouse_*/click/event`,
  `capture_screenshot -> RgbaImage` (`gpui/src/app/visual_test_context.rs:12-384`).
- Linux headless wgpu rendering with readback, no display server needed,
  merged 2026-09-25 (PR #64718); `gpui_linux/src/linux/headless.rs` gives
  windows that lay out and take input without drawing.
- Profiler: a bounded foreground journal of task polls, input dispatch, draws,
  presentations and frame skip reasons, plus a hang monitor
  (`gpui/src/profiler/journal.rs`, `profiler/hang/monitor.rs`). Useful as the
  base for "we already know what happened" crash logs.
- gpui-kit's harness: `window.find(id).role()/label()/value()/focused()/bounds()`,
  `click/input/press/hover`, `within(scope)`, `pointer_down/move/up`, `wait_for`
  ([gpui-kit TESTING.md](https://github.com/longbridge/gpui-kit/blob/main/crates/kit/TESTING.md),
  Context7 `/longbridge/gpui-kit`). This is close to our current Locator API.
- Missing: real OS-level input injection, multi-process automation socket,
  frame recording. ndshell's role (real cursor, TCC, screen capture) stays ours.

## 8. Performance on weak hardware

- **Idle:** demand-driven. No dirty view means the frame is skipped
  (`window.rs:1920-1933`); inactive windows cap at about 30 fps. The
  youtubemusic budget targets 0% CPU paused and unfocused on e1504g
  (`~/Developer/youtubemusic/PLAN.md:66-75`), which the user already pursues on
  gpui-pre.
- **Per-frame CPU:** upstream re-renders the whole view tree of any notified
  view and re-runs taffy layout. gpui-fast measured on Linux release builds:
  unchanged window 3.31 ms to 0.19 ms, sidebar scrolling 5.95 to 0.79 ms,
  streaming quotes 5.06 to 1.35 ms ([gpui-fast README](https://github.com/longbridge/gpui-fast)).
  On an Intel iGPU at 60 Hz (16.6 ms budget) upstream is fine for static UI;
  the retained-mode numbers matter for lists and live data.
- **Memory:** wgpu GL backend initialisation maps Mesa GL and LLVM, about
  59 MB RSS (zed PR #63313, open; patched locally in youtubemusic).
  Embedded fonts were stored twice until PR #62192 (2026-08-06).
- **Startup:** one report of a 45 s first window on an Intel i5-6300U under
  Wayland ([zed#58775](https://github.com/zed-industries/zed/issues/58775),
  closed stale). iGPU/dGPU switching freezes in
  [zed#52281](https://github.com/zed-industries/zed/issues/52281). Shader
  pipeline creation is the usual suspect; precompiled/`runtime_shaders`
  choices need measuring on e1504g.
- No published numbers exist for GPUI idle CPU, RSS or cold start on an Intel
  iGPU. We have to measure ourselves (messages has `scripts/linux-headless.sh`
  printing RSS and CPU under a headless sway).

## 9. Component ecosystems

- **gpui-kit** ([longbridge/gpui-kit](https://github.com/longbridge/gpui-kit),
  17k stars, 0.7.3 on 2026-10-12, docs at [gpui-kit.com](https://gpui-kit.com)):
  `gpui-base` (unstyled behaviour: focus trap, popup positioner, motion,
  reduce-motion, text selection, undo history, nav stack, virtual list, table,
  dock) and `gpui-component` (75+ styled: buttons, inputs, select/combobox,
  date/time, dialog/sheet/popover/hover card, menu + native_menu, dock, tabs,
  sidebar, tree, table, charts/plot, markdown/HTML text, code editor with
  tree-sitter + LSP, notification, title_bar, window_border, settings, speech).
  Also wasm, AccessKit roles, headless test harness, `gpui-shell` JS extension
  runtime, agent skills (`npx skills add longbridge/gpui-kit`). Used in
  production by Longbridge Pro. Pins `gpui-pre =0.3.9` exactly
  (`gpui-kit/Cargo.toml:68`).
- **gpui-macos** (Rhein-Industries, 0.1.x): macOS 27 look on gpui-base, real
  NSToolbar, SF Symbols, NSGlassEffectView, metrics measured against real
  AppKit controls (`docs/comparison.md`). One-week-old, single maintainer.
- **gpui-omarchy** (huacnlee): same pattern for an Omarchy/Hyprland look,
  referenced by gpui-macos's README.
- Others on crates.io: `herogpui` (HeroUI port), `gpui-router`, `gpui-tray`,
  `gpui-updater`, `gpui-devtools`, `gpui-fps`, `gpui-libghostty` (terminal),
  `gpui-pdf`, `gpui-bidi`, `native-theme-gpui`, `gpui-pre-mobile` (iOS/Android).
- **Missing for NativeDesktop parity:** a CEF view, Wayland webview,
  Liquid Glass upstream, haptics, trackpad swipe/rotate on Linux, kinetic
  scrolling on Linux, native tab groups (NSWindow tabbing is exposed as
  `SystemWindowTab` on mac only), matugen theming, and an external automation
  protocol.

## Verdict for NativeDesktop

GPUI is a sound Linux and Windows core and a credible macOS core. It already
gives us per-display pacing on mac and Wayland, AccessKit on three desktops,
IME everywhere, `list` with `FollowMode::Tail`, traffic light positioning, CSD
and drag regions, and a headless test story. The risks are channel and churn,
not capability.

Recommendations:

1. Build on `gpui-pre` pinned to an exact snapshot (`=0.3.x`), re-exported from
   one `nd-gpui` crate so apps never name GPUI directly. Bump weekly in one PR
   gated by our own test suite. Do not use crates.io `gpui` 0.2.2.
2. Run gpui-fast as a second backend behind a feature from day one (gpui-kit
   already does this with additive features). It carries the two things we
   need most: retained mode for e1504g and window composition for CEF and
   webviews. Drop it once #62379 and the retained work land upstream.
3. Take `gpui-base` as the behaviour layer and write our own presentation per
   platform (gpui-macos and gpui-omarchy prove the pattern). Skip
   `gpui-component`'s look; keep its table, editor and dock if needed.
4. macOS: do glass and toolbar the gpui-macos way (AppKit underlays and
   NSPanel popups via `raw_window_handle`), and add `NSHapticFeedbackManager`
   plus `CADisplayLink` frame-rate hints ourselves.
5. Own virtualization on top of `ListState`: a wrapper that enforces the
   "heights outside the viewport don't change without splice" rule, with tests
   for tail-follow, prepend anchoring and the last-row case. gpui-kit's
   `message_scroller` is the reference.
6. CEF: start with composition (gpui-fast surfaces) for native speed; keep an
   OSR texture fallback for Wayland until a subsurface path is proven.
7. Measure on e1504g in week one: cold start, idle CPU, RSS with the GL backend
   patched out, scrolling a 10k-row list. Measure g815 240 + 144 Hz and
   mac ProMotion with the profiler journal.

Risks:

- **Single-vendor ecosystem.** gpui-pre, gpui-fast, gpui-kit, gpui-webview and
  the composition PR all come from Longbridge (huacnlee). If that stops, we
  inherit snapshot publishing.
- **Upstream churn.** Weekly breaking changes; Zed prioritises Zed. Our
  `nd-gpui` facade is the firewall.
- **Wayland gaps.** No webview, no kinetic scroll, pinch only, blur only on KDE,
  mixed refresh issues on GNOME.
- **Windows vsync** follows DWM, not the window's monitor; unverified on mixed
  240/144 Hz.
- **Web target** is single window and experimental; treat docs-site-in-GPUI as
  a stretch goal, not a launch requirement.
