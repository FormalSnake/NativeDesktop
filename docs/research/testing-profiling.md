# Testing, profiling and diagnostics: research for the Rust rewrite

Date: 2026-10-13. Scope: in-process and out-of-process testing, isolation, profiling, flight
recorder, build speed. Tags: [local] read on this machine, [src] read from upstream source today
via `gh api`, [doc] fetched docs/specs today, [crates] crates.io API today, [web] secondary
source, [mem] from memory and not re-verified (treat as a lead).

Backend reminder from PROMPT.md: AppKit on macOS, GPUI on Linux, WinUI (or GPUI) on Windows. So
"GPUI test contexts" cover the Linux backend and the shared element layer; AppKit and WinUI each
need their own pixel path.

## 0. What the old (Zig/Bun) stack already got right, and what hurt

- `@nativedesktop/test` already gives each launch its own temp `XDG_RUNTIME_DIR` and parses its
  own automation socket path from stderr [local: `packages/test/src/launch.ts:108,249`]. Keep
  that shape: per-launch dirs, socket path reported by the child, never a fixed path.
- Locators (`getByRole/TestId/Text`, `>>` chaining, actionability ladder), `waitFor` evaluated
  host-side on the retained tree, dialog scripting, and the 3-tool MCP bridge
  (`execute/snapshot/reset`) are the surface worth porting 1:1 [local: CLAUDE.md, automation
  sections].
- `tools/ndshot` (Swift) carries the macOS hard parts: SCK region capture, focus via SkyLight,
  `--nd-grant` TCC.db rows with SIP off, `--nd-input` HID CGEvent helper, responsibility
  disclaim [local: `tools/ndshot/Sources/*.swift`]. All of it ports to Rust through `objc2`.
- What hurt: every real-window test fights over one window server, one focus, one cursor; and
  the AppKit screenshot path needed fixes for stale bitmap reps ("ghosting") [local: CLAUDE.md].

## 1. In-process testing (GPUI and the shared layer)

### 1.1 Test contexts available upstream [src: zed main, crates/gpui/src/app/*]
- `TestAppContext` (`#[gpui::test]`): mocked `TestPlatform`, deterministic single-threaded
  `TestDispatcher`. Helpers include `simulate_keystrokes/input/click/mouse_*`, `simulate_event`
  (any `InputEvent`), `simulate_window_resize`, `simulate_scale_factor_change`,
  `simulate_display_added/removed`, `simulate_display_refresh_interval_change`,
  `simulate_prompt_answer`, `simulate_path_prompt_response`, `debug_bounds`, `run_until_parked`,
  `advance_clock` [src: `app/test_context.rs:392-633`]. Multi-monitor and refresh-rate changes
  (60/144/240/120 Hz) are therefore testable without any monitor.
- `HeadlessAppContext`: platform-neutral, `TestPlatform` scheduling plus a real
  `PlatformTextSystem` (CoreText, DirectWrite, cosmic-text) for exact glyph metrics, and an
  optional renderer factory enabling `capture_screenshot(window) -> RgbaImage`
  [src: `app/headless_app_context.rs:1-35,168`]. This replaced the macOS-only
  `HeadlessMetalAppContext`.
- `VisualTestAppContext`: real platform, `open_offscreen_window`, `capture_screenshot` via
  `window.render_to_image()`; no Screen Recording permission needed because it reads the render
  pipeline, not the screen [src: `app/visual_test_context.rs:97,384`; doc: zed PR #45259, merged
  2026-01-05, runner `cargo run -p zed --bin visual_test_runner --features visual-tests`,
  `UPDATE_BASELINE=1` to rebless].

### 1.2 Real pixels without a window server
- macOS: Metal headless renderer exists (`render_to_image` into a non-framebuffer-only texture)
  [doc: PR #45259]. Works on a logged-in Mac without a visible window.
- Linux: zed PR #64718 "gpui_wgpu: Add Linux headless rendering", merged 2026-09-25: surface-less
  wgpu path, offscreen target reuse, RGBA readback, verified with `DISPLAY` and
  `WAYLAND_DISPLAY` unset. It rejects software adapters by default with an explicit
  software-allowed constructor for functional tests [doc: github.com/zed-industries/zed/pull/64718].
  The superseded #62341 showed llvmpipe via Vulkan (lavapipe) producing byte-equal output across
  runs [doc: PR #62341].
- Published crates lag: `gpui-pre-platform` 0.3.3 returns `MetalHeadlessRenderer` on macOS and
  `None` elsewhere, so screenshot tests panic on Linux ("no HeadlessRenderer configured")
  [web: github.com/Syynth/brink/issues/3692, 2026-10-10]. youtubemusic/messages pin
  `gpui-pre` 0.3.6 and already patch `gpui-pre-wgpu` [local: `youtubemusic/Cargo.toml:43`].
  NativeDesktop must pin a zed rev containing #64718 (or vendor it).
- Windows: PR #64718 does not mention Windows; #62341 wired wgpu headless for Linux and Windows
  but was closed unmerged [doc]. Treat Windows headless GPUI as unproven.
- CI software GPU: Mesa lavapipe (`mesa-vulkan-drivers`) gives `llvmpipe` on Vulkan; wgpu picks
  it with `compatible_surface: None` [web: brink#3692; doc: PR #62341]. Text from cosmic-text
  differs from CoreText, so golden images are per-platform, never shared [web: brink#3692].
- AppKit backend (not GPUI): `NSView.cacheDisplay(in:to:)` into
  `bitmapImageRepForCachingDisplay` renders an off-screen window's views without capture
  permission [mem]; old ND used this and hit stale-rep bugs [local]. Glass/vibrancy materials
  are composited by the window server and will not appear in a view-cache; they need an SCK
  capture (section 2). WinUI: `RenderTargetBitmap.RenderAsync` renders a XAML subtree, with the
  same caveat for Mica/Acrylic [mem].

### 1.3 Deterministic clock and frame-by-frame animation capture
- `TestClock` (scheduler crate) advances `now` and `utc_now` on `advance_clock`; the executor's
  `now()` reads it [src: `crates/scheduler/src/clock.rs`].
- Catch: GPUI's `with_animation` only uses the executor clock for `synced` animations; normal
  ones use `state.start.elapsed()` on `web_time::Instant` (wall clock), and springs use
  `Instant::now()` [src: `elements/animation.rs:271,403,420-434`]. So stock GPUI animations are
  not deterministic under `advance_clock`.
- Consequence for NativeDesktop: route every framework animation, spring and gesture-momentum
  through one `cx.now()` frame clock (the executor clock in tests, the display-link timestamp in
  production). Then "capture frame N of a 250 ms ease at 120 Hz" is `advance_clock(1/120 s)` +
  `capture_screenshot`, looped: exact, repeatable, no screen recorder, runs in parallel.
- Timer caveat: tasks must use GPUI's executor timers, not `smol::Timer`, or `run_until_parked`
  reports idle early [web: gpui-test skill pages; `DEBUG_SCHEDULER=1`, `PENDING_TRACES=1`].

## 2. Out-of-process driving of real windows

In-process tests cover logic and pixels; out-of-process "real use" tests cover what only the OS
does: resize handles, traffic lights, menus, popovers leaking input, CEF child windows, IME,
trackpad gestures, focus. Keep these few and serialize them per display (section 3).

### 2.1 macOS
- Input: posted CGEvents. `CGEventPostToPid` reaches only the app owning the menu bar for keys
  and still needs focus for dialogs [web: developer.apple.com/forums/thread/695941, /724835].
  So real-input tests must bring the window forward first (old ndshot's SkyLight front-process +
  make-key records [local]) and run one at a time per login session.
- Capture: ScreenCaptureKit, window or region, composited with sheets/menus; `minimumFrameInterval`
  sets the cap. Apple samples use 60 fps; 120 fps on ProMotion is unconfirmed (one report of a
  hard 60 fps cap on iPadOS 27 beta) [web: developer.apple.com/forums/thread/848296]. For
  animation work prefer the in-process frame clock (1.3); use SCK to verify composited reality.
- TCC with SIP off: write Screen Recording, Accessibility, PostEvent rows keyed to the signing
  identifier; TCC matches the real path, not SwiftPM/cargo symlinks [local: CLAUDE.md
  "Composited captures"]. Keep the capture/input helpers as subcommands of the host binary
  (`--nd-capture`, `--nd-input`) so one grant covers everything.
- Multi-monitor without monitors: private `CGVirtualDisplay`/`CGVirtualDisplayMode`
  (`initWithWidth:height:refreshRate:`) creates displays with a chosen refresh rate
  [src: chromium `ui/display/mac/test/virtual_display_util_mac.mm:76-87`]; Electron ships this as
  `@electron-ci/virtual-display` for its CI [doc: electronjs.org multi-monitor-testing]. Lets a
  MacBook test 144 Hz + 240 Hz window moves. Private API: test-only, never in shipped apps.

### 2.2 Linux (Hyprland / wlroots-style)
- Input: `zwlr_virtual_pointer_v1` and `zwp_virtual_keyboard_v1`; Hyprland also has an EIS
  (libei) server [local: `Hyprland-src/protocols/`, `src/managers/input/Eis.cpp`].
- Capture: Hyprland implements `ext-image-copy-capture-v1` and `wlr-screencopy`
  [local: `Hyprland-src/src/managers/ProtocolManager.cpp:232`, `CMakeLists.txt:608`].
  Presentation feedback (`wp_presentation`) gives per-frame present timestamps [local].
- Isolation: Hyprland's own test runner launches it with `HYPRLAND_HEADLESS_ONLY=1` and creates
  outputs with `hyprctl output create headless HEADLESS-2` [local:
  `Hyprland-src/hyprtester/src/main.cpp:70,272`]. Each instance gets its own
  `HYPRLAND_INSTANCE_SIGNATURE` and `WAYLAND_DISPLAY` under `$XDG_RUNTIME_DIR/hypr`
  [local: `src/Compositor.cpp:192-208,417`]. The aquamarine backend list is headless (mandatory),
  DRM (if available), Wayland (fallback, i.e. nested) [local: `Compositor.cpp:303-318`].
  A per-test headless Hyprland is a private seat, focus and cursor: real input tests run in
  parallel with no VM. Needs GPU or render-node access; ssh sessions lack it [web:
  forum.hypr.land/t/uwsm...146].
- Fallbacks: sway with `WLR_BACKENDS=headless` [mem]; weston headless backend (renderer support
  for GL on headless is a long-standing gap) [web: gitlab.freedesktop.org/wayland/weston/-/issues/278].
  cage is a kiosk compositor, useful for a single fullscreen app, not multi-window tests [mem].

### 2.3 Windows
- Input: `SendInput` targets the foreground input desktop only [mem]; one real-input test per
  interactive session. UI Automation (UIA) gives a tree WinUI already populates [mem].
- Capture: Windows.Graphics.Capture via `windows-capture` 2.0.1 (window or monitor,
  minimum update interval, cursor/border toggles, hardware encoder to MP4)
  [doc: docs.rs/windows-capture; crates: 2026-08-08].
- Isolation: no headless compositor. Real-input tests on Windows run serialized on the g815
  over `ssh windows` [local: rules/windows-host.md]; everything else runs in-process.

### 2.4 Accessibility tree, locators, MCP
- GPUI now integrates AccessKit: elements expose a `role()` (nodes without a role are not
  reported), NodeIds derive from `GlobalElementId`, actions come from `accesskit::Action`
  [src: `crates/gpui/src/_accessibility.rs:15-230`, updated 2026-06-17]. `accesskit` 0.25.1
  [crates]. AppKit and WinUI publish native AX/UIA trees.
- Recommendation: the automation socket serves NativeDesktop's own retained tree (role, name,
  value, test_id, bounds, enabled/focused/checked/expanded) built from the same data that feeds
  AccessKit / NSAccessibility / UIA. One tree, one schema on all backends; querying the OS AX API
  is only a cross-check. Port the old selector grammar and actionability ladder unchanged.
- Transport: Unix socket (named pipe on Windows), path chosen by the launcher and passed in env,
  reported back on stderr. `waitFor` evaluated host-side, as before.
- MCP: `rmcp` 3.5.1 is the Rust MCP SDK [crates: updated 2026-10-05]. Keep the 3 tools
  (`execute`, `snapshot`, `reset`); `execute` can run a small Rust-free script language or JS via
  an embedded runtime, or keep the TS harness as a thin client over the same socket.

## 3. Isolation so apps and agents test concurrently

| Shared resource | Fix |
| --- | --- |
| Automation socket | Per-launch path in a per-test temp dir; never a fixed path or TCP port (nextest docs advise port 0 or Unix sockets) [doc: nexte.st test-groups] |
| App data / settings / logs / crash dir | `ND_APP_DATA_DIR` override, set to the test temp dir by the harness; every path API (data dir, store, logs, crash bundles) resolves through it |
| `XDG_RUNTIME_DIR`, D-Bus, keyring | Per-test temp `XDG_RUNTIME_DIR` [local: launch.ts:108]; mock credentials backend in tests |
| Window server, focus, cursor | Linux: one headless Hyprland per test. macOS/Windows: nextest test group `os-input` with `max-threads = 1` [doc: nexte.st test-groups] |
| Process-global state | nextest runs each test in its own process [doc], so statics and `OnceLock` can't leak between tests |
| Single-instance locks | Lock file inside the app data dir, so per-test dirs never collide |
| TCC grants (macOS) | Granted once to the host binary's signing id; tests never prompt |
| Build dir between agents | Shared cargo `build-dir` on the external drive, per-worktree `target-dir` (section 6) |

GPUI's real `MacPlatform` must own the main thread, so real-platform tests on macOS live in
`harness = false` test binaries (zed does this with `visual_test_runner`) [doc: PR #45259].

## 4. Profiling

| Tool | Version | Use | Release-build cost |
| --- | --- | --- | --- |
| `tracy-client` / `tracing-tracy` | 0.19.0 / 0.12.0 [crates] | Live CPU/GPU zones, frame marks, plots, allocations | `ondemand` feature: collection starts only when a profiler connects [doc: docs.rs/tracy-client]. Zone ~8 to 15 ns when connected [web: tracy manual commits]. Listens on the network and can broadcast; `only-localhost` restricts it [doc] |
| `puffin` + `puffin_http` | 0.20.0 / 0.17.0 [crates] | In-app flamegraph, cheap scopes | 1 ns per scope when off, 50 to 200 ns on [doc: github.com/EmbarkStudios/puffin] |
| Perfetto | `perfetto-sdk` 1.1.1 [crates]; `tracing-perfetto` stale since 2025-01 | Chrome-trace/Perfetto UI for startup traces | Write-to-file on demand only |
| `wgpu-profiler` | 0.28.0 [crates] | GPU timestamp queries per pass, tracy/chrome export | Off by default; enable per session |
| GPUI profiler | in tree | Foreground journal ring (task polls, actions, input, draws, presents), `HangDetector` with frame-budget rule, hdrhistogram timings | Feature `profiler` [src: `gpui/src/profiler.rs`, `profiler/journal.rs`, `profiler/hang.rs`] |

- Zed's pattern: a `ztracing` facade that compiles spans to nothing unless built with
  `--cfg ztracing`, with `tracing-tracy` + `tracy-client` (`ondemand`) and an optional tracy
  `ProfiledAllocator` [src: `crates/ztracing/{Cargo.toml,src/lib.rs}`].
- Frame timing per monitor: present timestamps per window from the display it is on
  (CADisplayLink per screen on macOS 14+ [mem], `wp_presentation` feedback on Wayland [local],
  DXGI frame statistics on Windows [mem]). GPUI's journal already records `PresentTiming` per
  window [src: journal.rs]. Report budget misses against that display's interval, so a window
  dragged from the 240 Hz to the 144 Hz panel switches budget (4.17 ms to 6.94 ms).
- Startup tracing: one span tree from `main` to first present, written as a chrome-trace file
  when `ND_TRACE_STARTUP=1`; Perfetto UI opens it.
- Always-on in release: `tracing` events at info+ into the flight recorder ring, the GPUI-style
  journal and hang detector, frame-time histograms. On demand: tracy (`ondemand`, localhost),
  puffin server, GPU timestamps. Compiled out: trace-level spans in hot loops.

## 5. Flight recorder (no reproduce-to-debug)

### 5.1 Pieces
- Structured logs: `tracing` + `tracing-subscriber` 0.3.23 [crates]. Disk: `tracing-appender`
  0.2.5 rolling `Builder` with `rotation`, `filename_prefix/suffix`, `max_log_files(n)`
  (deletes oldest matching files on start and on roll; set m+1 to keep m) and `latest_symlink`;
  time-based only [doc: docs.rs/tracing-appender rolling::Builder]. Need size caps too:
  `logroller` 0.1.12 adds size+age rotation, `max_keep_files`, gzip, and feeds
  `tracing_appender::non_blocking` [doc: docs.rs/logroller].
- In-memory ring: a `tracing` layer keeping the last N events (structured, not formatted) in a
  bounded buffer, flushed only on error/crash/hang. Cheap enough for debug level.
- Crash capture: `crash-handler` 0.8.1 + `minidumper` 0.11.0 [crates]. Zed's `crashes` crate
  runs a minidumper server subprocess over a socket, with ping/connect timeouts, installs signal
  handlers lazily, and a panic hook that forces backtraces and suppresses the macOS crash dialog
  [src: `crates/crashes/src/crashes.rs`]. `sentry-rust-minidump` 0.18.1 wraps the same two
  crates and relaunches the executable as its own crash reporter [crates; web: crates page].
  `sentry` 0.49.3 + `sentry-tracing` for upload [crates]; the user already runs Sentry EU
  (`canarycoders`) for other apps [local: rules/sentry-auth-token.md].
- Hangs: GPUI's `HangDetector` reports any foreground work over a threshold, or intervals whose
  total spend blew the frame budget [src: profiler/hang.rs].

### 5.2 What a crash/error bundle contains
One directory per incident under `<app data>/diagnostics/<timestamp>-<kind>/`:
1. `minidump.dmp` (native crash) or `panic.txt` with backtrace (panic) or `hang.json`.
2. `log.ndjson`: the in-memory ring (last ~5000 events) plus the tail of the current log file.
3. `tree.json`: the retained UI tree at failure (same schema as the automation `getTree`),
   windows, displays with refresh rates, focus, open popovers/menus.
4. `input.ndjson`: last ~200 input events after the framework's own dispatch (kind, target
   node, position, modifiers, timestamps). Redact text keystrokes in release unless opted in.
5. `frames/`: last ~3 to 5 presented frames per window as downscaled PNG, taken from a GPU ring
   of readback textures that is only read on incident (zero cost in steady state).
6. `profile.json`: last ~120 frame timings per window and the GPUI journal tail.
7. `meta.json`: version, git sha, OS, GPU adapter, backend, config hash.

On panic, write 2 to 6 from the panic hook in the same process (UI thread may be the panicking
one, so tree/frames come from the last committed snapshot, never by re-rendering). On native
crash, the minidumper server writes the dump and the main process's last periodic snapshot
(written every few seconds to a memory-mapped file) supplies 3 to 6.

### 5.3 Retention
- Logs: daily + 10 MB size roll, keep 7 files, gzip rotated ones.
- Incident bundles: keep the newest 20 and nothing older than 14 days; total cap 200 MB,
  oldest deleted first. Cleanup runs at startup on a background thread, never blocking first
  frame.
- `nd diagnose` (CLI) zips the newest bundle for an agent or an issue; `nd diagnose --last`
  prints the summary so an agent reads it without reproducing.

## 6. Build and test speed

- Runner: `cargo-nextest` 0.9.148 [crates]. Process per test (isolation for free), test groups
  with `max-threads` as a cross-process mutex, `threads-required`, priorities, `slow-timeout`
  [doc: nexte.st]. Setup scripts can export env (`$NEXTEST_ENV`) but are still experimental
  (`experimental = ["setup-scripts"]`) [doc: nexte.st setup-scripts]. Zed's CI config uses
  groups, priorities and per-test timeouts exactly this way [src: zed `.config/nextest.toml`].
- Codegen: Cranelift via `rustup component add rustc-codegen-cranelift-preview --toolchain
  nightly`, set per profile (`codegen-backend = "cranelift"`); supported on Linux x86_64/aarch64,
  macOS x86_64/aarch64, Windows x86_64 [src: rustc_codegen_cranelift README]. Limits: partial
  `std::arch` SIMD, and panics abort (no unwinding) on macOS and Windows, which breaks
  `#[should_panic]` and `catch_unwind`-based tests [src: same README]. Use it for `nd dev`
  builds of the workspace crates, keep LLVM for test profiles on macOS. A 2026 project goal
  targets 2x faster incremental cg_clif [web: rust-lang.github.io/goals/2026].
- Linkers: Linux `wild` (0.10, Aug 2026; Linux-only, Mach-O "can link some very simple
  programs", no incremental linking yet) [web: phoronix Wild 0.10, rustfoundation.org]. mold on
  Linux is the stable fallback [mem]. macOS: Apple's default `ld` (ld-prime) [mem]. Windows:
  `rust-lld` [mem].
- Caching: `sccache` 0.18.0 [crates] caches only non-incremental crates, never bin/cdylib/
  proc-macro, so it helps third-party deps across worktrees and nothing else
  [web: Android sccache docs, cargo#8143]. With many worktrees, a shared `build-dir`
  (`CARGO_BUILD_BUILD_DIR`, separated from `target-dir` in Cargo 1.91) on the external drive
  does more [doc: blog.rust-lang.org 2026-03-13 build-dir layout v2].
- Test binary shape: one integration-test binary per crate (`tests/it/main.rs` with modules)
  instead of one per file, so a change relinks one binary [mem]. Unit tests in the crate they
  test. Keep the GPUI/AppKit/WinUI backends out of core crates so core tests never link them.
- Dependencies at `opt-level = 2` in dev, workspace at 0 (youtubemusic already does this)
  [local: `youtubemusic/Cargo.toml` `profile.dev.package."*"`].
- Targets for "under seconds": core logic test edit-to-result under 3 s (incremental + nextest
  filter), headless pixel test under 1 s per test after build, full in-process suite under 60 s
  in parallel, real-window suite only on demand.

## Verdict for NativeDesktop

1. Three test tiers, one API. Tier 1: in-process, `TestPlatform`-style deterministic contexts
   for logic, layout, input routing, multi-monitor and refresh-rate changes; runs fully
   parallel, no window server. Tier 2: in-process headless pixels (Metal headless on macOS, wgpu
   headless on Linux with lavapipe in CI) for golden images and frame-by-frame animation
   capture. Tier 3: out-of-process real windows over the automation socket, only for OS-owned
   behaviour. The locator/expect API is identical across tiers; tier 3 adds `cursor`, `capture`.
2. Own the frame clock. Every animation, spring and momentum reads `cx.now()`; upstream GPUI's
   wall-clock animations are the one thing that would make tier 2 animation capture flaky.
3. Pin a zed rev with PR #64718 (Linux headless, merged 2026-09-25); published `gpui-pre`
   crates lack it. Windows headless GPUI is unproven; budget a patch or run tier 2 on Win only
   via WinUI `RenderTargetBitmap`.
4. Parallel real-window tests on Linux via one `HYPRLAND_HEADLESS_ONLY=1` Hyprland per test
   (its own seat, virtual pointer/keyboard, ext-image-copy-capture). macOS and Windows tier 3 is
   serialized through a nextest `os-input` group; `CGVirtualDisplay` gives fake 144/240 Hz
   displays on the MacBook.
5. One retained tree feeds AccessKit/NSAccessibility/UIA and the automation socket; locators
   query that tree, not the OS AX API. MCP bridge on `rmcp` with the old 3 tools.
6. Isolation by construction: every path (socket, data, logs, crash dir, locks) resolves from a
   per-launch root the harness sets; nextest process-per-test kills global-state leaks.
7. Profiling: `tracing` everywhere behind a facade (zed's `ztracing` pattern); tracy
   `ondemand`+`only-localhost` built into dev and opt-in release builds; GPUI-style journal,
   hang detector and per-display frame histograms always on.
8. Flight recorder always on: in-memory event ring, rolling logs (logroller: size+age, gzip,
   keep 7), crash-handler + minidumper subprocess (zed `crashes` pattern), incident bundles with
   tree + last inputs + last frames + timings, capped at 20 bundles / 14 days / 200 MB.
9. Speed: nextest, Cranelift for dev runs (LLVM for macOS tests because of panic=abort), wild on
   Linux, shared `build-dir` on the external drive, one test binary per crate, deps at
   opt-level 2. sccache is optional, not the answer.
