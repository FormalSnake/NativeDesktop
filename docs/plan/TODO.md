# NativeDesktop Rust port: todo

Status legend: `[ ]` open, `[~]` in progress, `[x]` done and verified.

## Phase 0: setup
- [x] Old main renamed to `zig`, orphan `main` at `~/Developer/ecosystem/nativedesktop`
- [x] Record verbatim prompt (`docs/plan/PROMPT.md`)
- [x] Build cache dir on `/Volumes/Kyan Drive/dev-cache.noindex/`
- [ ] Move nativebrowser, FormalShell, nothingctl, meshtop, messages, youtubemusic into `~/Developer/ecosystem/` once their panes are idle; `git worktree repair` after
- [ ] Repo-wide cargo config pointing target dir + sccache at the external drive; nix dev shell

## Phase 1: research (reports in `docs/research/`)
- [x] GPUI: platforms (macOS, Linux Wayland/X11, Windows, web), multi-monitor refresh, a11y, IME, gestures, virtualization, blur, native view embedding (JaceThings tweet), gpui-component, gpui-kit.com
- [x] macOS: AppKit + Liquid Glass hybrid, traffic light placement, toolbars, menus, haptics, CEF in-window
- [x] Windows: WinUI 3 from Rust vs GPUI Windows, Mica, installer, auto-update
- [x] Linux + Nix: matugen, Hyprland, home-manager module, config format choice
- [x] Testing/ndshell: input simulation, frame recording, SIP, profiling, crash logs, all platforms
- [x] Parity inventory: every feature of the `zig` branch (`docs/parity/zig.md`)
- [x] App inventory: repeated patterns and handrolled components in messages, youtubemusic, nativebrowser, GPUI apps (`docs/parity/apps.md`)

## Phase 2: architecture
- [x] `docs/plan/ARCHITECTURE.md` drafted (AppKit on macOS, GPUI on Linux, GPUI Fluent skin on Windows, GPUI on web; one GPUI-shaped vocabulary)
- [x] Design-target example apps in `docs/plan/examples/` (hello, notes, native-widget, overrides)
- [x] Adversarial review (`docs/plan/REVIEW.md`, 35 findings) resolved into ARCHITECTURE.md and the examples; see the Resolution section there
- [x] Open questions answered (ARCHITECTURE.md section 17), except signing (decided at M10)

Milestones below match ARCHITECTURE.md section 16. Each closes only when `just gate mN` passes on the machines named there.

## Phase 3: foundation
- [ ] M0 day one, before lanes fork: backend seam spec + `TestBackend` (3.15); dispatch spike: nd dispatch on `TestBackend`, `NdAppKitPlatform` owning NSApplication, a binding firing while a native NSTextField has focus, toolbar item in the key view loop (3.12)
- [ ] M0 skeleton and first mac window: workspace, cargo config on the external drive with mount guard, nix dev shell without SDKROOT on darwin, `swift-toolchain.toml`, CI, `nd-core`, `nd-gpui` + maintained fork with rebase gate, `nd-ui` core (call-site identity, event queue, Taffy), `nd-backend-appkit` (NSWindow, primitives, native button, `nd::build::app()` Swift aggregate), traffic lights by coordinates, `examples/hello`, stub `nd` crate type-checking all examples, raw gpui-pre baseline on e1504g, LLVM vs Cranelift dev build measured; gate: tier 1 + 2 green, examples `cargo check` against the stub, real HID edge drag with SCStream-captured frames matching the window edge
- [ ] M1 diagnostics and harness: frame clock, `nd-diag` (bundles, retention, hang watchdog), frame recorder (macOS layout diffs, GPUI last frame), semantic tree, `nd-automation`, `nd-test` locators, `ndshell` machine lock, `nd-mcp`, minimal `nd new/check/test`, `subsecond` spike; examples move to `examples/` with per-example `required-features`; gate: complete bundle, tier 3 locator click, MCP snapshot, retention, concurrent runs queue on the lock
- [ ] M2 GPUI backend on Linux and Windows: `nd-backend-gpui` with the dispatch hook, `nd-os-linux` (xdg-decoration, Hyprland no controls), `nd-os-windows` (Mica, caption-button spike, Snap Layouts, WM_POINTER), per-window clocks, occlusion throttling, drag regions, window controls, `titlebar()` insets, multi-window, headless Hyprland runner, Windows `ndshell agent`; gate: real-drag resize on three OSes, `nd diag pacing` (g815 240 + 144 Hz; Windows real or virtual pair), e1504g budgets with and without `fast`

## Phase 4: look, input, lists, components
- [ ] M3 theme, skins, patterns: tokens resolved from the OS, Fluent skin, `glass()`, matugen watcher + template, intents, override layers; gate: goldens for Linux and macOS looks, Fluent goldens and metric comparison on the Windows host, matugen live recolour
- [ ] M4 input, overlays, native surfaces: gestures on both backend kinds, haptics, focus rules, in-window overlays on macOS, promotion on GPUI, `NativeSurface` with minimal WKWebView and WebView2 composition hosts, Linux test GPU surface; gate: overlay leak test over a native surface on three OSes, resize gate with webview and GPU surface, pinch and page swipe on macOS
- [ ] M5 virtualization: `nd-virtual` with height tree, row cache and versions, NSTableView/NSOutlineView/NSCollectionView presenters (AppKit estimation off), GPUI presenter; gate: conformance suite on GPUI (tier 1) and a real NSTableView (tier 2), 100k rows on e1504g
- [ ] M6 components: parity widgets, zig `ui` set, ranked app components, Spotlight-style command palette, Swift-hosted leaves, echo and reentrancy tests per widget; gate: tier 1 behaviour + tier 2 AppKit mapping test per component, goldens, role audit, gallery
- [ ] M7 settings, config, system: `nd-config`, `nd-settings` + `settings_form` + `config.d` + HM generator, `Store<T>`, `nd-system` (incl. readImage), `nd-ipc`, `nd-data`, `Plugin` with `[plugins.*]` + Discord RPC example; gate: comment-preserving round trip, Nix read-only, plugin section never breaks strict parse, `nix flake check`

## Phase 5: web content and terminal
- [ ] M8 webviews: full WKWebView, WebView2 composition with input forwarding and UIA, WPE spike then engine, engine matrix; gate: every M8 matrix cell, overlay leak and resize with a live page
- [ ] M9 CEF and terminal: OSR with macOS IOSurface pool, external begin frame, input/IME/select/menus/drag, Chrome-style (XWayland on Hyprland), helpers, adblock, `nd-terminal`; gate: display-Hz pacing, every M9 matrix cell, zero Chromium bytes without CEF

## Phase 6: tooling, web, docs
- [ ] M10 CLI, packaging, updates: `nd new` full (build script in every template), `nd dev` with `subsecond`, `nd package` (mac/win/linux, signing, branded installer), `nd release` with publishers, `nd-update` signed feed, `mkApp`, README template; gate: `nd new demo` to package on each OS, signed delta update from `nd release serve`, tampered feed rejected, edit to visible change under 1.5 s
- [ ] M11 web backend: wasm `nd-backend-gpui` with the plain GPUI look, `nd-os-web`, capability checks, `nd build --target web`; gate: every example builds for wasm and its tier 1 tests pass under `wasm-bindgen-test` in headless Chromium, web goldens, capability lint
- [ ] M12 docs: `nd-docs` site, four-tab embeds, gallery, docs app, agent docs, OS-release diff job; gate: examples compile everywhere, `nd check` clean over examples, embeds load, no stale capture, link check

## Phase 7: ports
- [ ] M13 messages (can start after M7)
- [ ] M14 youtubemusic (after M7; daemon on `nd-ipc`)
- [ ] M15 nativebrowser (after M9; gate includes extensions and 1Password on Hyprland)
- [ ] Files/Finder prep along the way: grid/table at 100k entries, column view, file drag with promises, preview overlay, file watcher, thumbnails, volumes source list, breadcrumb
