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
- [ ] Adversarial review pass of ARCHITECTURE.md and the examples before any code
- [ ] Answers to the open questions in ARCHITECTURE.md section 17

Milestones below match ARCHITECTURE.md section 16. Each closes only when `just gate mN` passes on the machines named there.

## Phase 3: foundation
- [ ] M0 skeleton and first mac window: workspace, cargo config on the external drive with mount guard, nix dev shell, CI, `nd-core`, `nd-gpui`, `nd-ui` core (views, retained tree, reconciler, Taffy, TestBackend), `nd-backend-appkit` (NSWindow, layer/text primitives, native button, Swift bridge stub), traffic lights by coordinates, `examples/hello`; gate: tier 1 + 2 green, `hello --nd-selftest resize`
- [ ] M1 diagnostics and harness: frame clock, `nd-diag` (flight recorder, bundles, retention), frame recorder, semantic tree, `nd-automation`, `nd-test` locators, `nd-mcp`, minimal `nd new/check/test`; move `docs/plan/examples` to `examples/`; gate: seven-part bundle, tier 3 locator click, MCP snapshot, retention
- [ ] M2 GPUI backend on Linux and Windows: `nd-backend-gpui`, `nd-os-linux` (xdg-decoration, Hyprland no controls), `nd-os-windows` (Mica, DWM caption buttons, Snap Layouts, WM_POINTER), per-window display clocks on all OSes, drag regions, window controls, multi-window; gate: resize gate on three OSes, `nd diag pacing` on g815 240 + 144 Hz

## Phase 4: look, input, lists, components
- [ ] M3 theme, skins, patterns: tokens resolved from the OS, Fluent skin, `glass()`, matugen watcher + template, intents (scaffold, toolbar, sidebar and inspector split items, sheets, settings window, menus, alerts, pickers), override layers; gate: goldens per skin, Fluent vs WinUI Gallery, matugen live recolour
- [ ] M4 input: gestures on both backend kinds, haptics, focus and keyboard rules, overlays with promotion; gate: overlay leak test over a native surface on three OSes, pinch and page swipe on macOS
- [ ] M5 virtualization: `nd-virtual` model, NSTableView/NSOutlineView/NSCollectionView presenters, GPUI presenter; gate: shared conformance suite on both, 100k rows on e1504g
- [ ] M6 components: parity widgets and ranked app components on `NativeWidget`, Spotlight-style command palette ported from `CommandPalette.swift`, Swift-hosted leaves (Charts, Form rows, Gauge); gate: per-component tests, goldens, role audit, gallery
- [ ] M7 settings, config, system: `nd-config`, `nd-settings` + `settings_form` + HM module generator, `Store<T>`, `nd-system`, `nd-ipc`, `nd-data`, `Plugin` + Discord RPC example; gate: comment-preserving round trip, Nix read-only, `nix flake check`

## Phase 5: web content and terminal
- [ ] M8 native webviews: `NativeSurface`, WKWebView, WebView2 composition, WPE spike then engine; gate: webview suite, overlay leak with a live page, resize with webview
- [ ] M9 CEF and terminal: OSR, external begin frame, input/IME/select/menus/drag, Chrome-style mode, helpers, adblock, `nd-terminal`; gate: display-Hz pacing, OSR item tests, zero Chromium bytes without CEF

## Phase 6: tooling, web, docs
- [ ] M10 CLI, packaging, updates: `nd new` full, `nd dev`, `nd package` (mac/win/linux, signing, branded installer), `nd release`, `nd-update` signed feed, `mkApp`, README template; gate: `nd new demo` to package on each OS, signed delta update, tampered feed rejected
- [ ] M11 web backend: wasm `nd-backend-gpui` with the plain GPUI look, `nd-os-web`, capability checks, `nd build --target web`; gate: every example builds for wasm unchanged, web goldens, capability lint
- [ ] M12 docs: `nd-docs` site, four-tab embeds (live web, captured macOS/Linux/Windows), gallery, docs app, agent docs with the OS-structure rule; OS-release diff job; gate: examples compile everywhere, embeds load, no stale capture, link check

## Phase 7: ports
- [ ] M13 messages (can start after M7)
- [ ] M14 youtubemusic (after M7; daemon on `nd-ipc`)
- [ ] M15 nativebrowser (after M9)
- [ ] Files/Finder prep along the way: grid/table at 100k entries, column view, file drag with promises, preview overlay, file watcher, thumbnails, volumes source list, breadcrumb
