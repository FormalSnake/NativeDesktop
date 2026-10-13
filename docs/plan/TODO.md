# NativeDesktop Rust port: todo

Status legend: `[ ]` open, `[~]` in progress, `[x]` done and verified.

## Phase 0: setup
- [x] Old main renamed to `zig`, orphan `main` at `~/Developer/ecosystem/nativedesktop`
- [x] Record verbatim prompt (`docs/plan/PROMPT.md`)
- [x] Build cache dir on `/Volumes/Kyan Drive/dev-cache.noindex/`
- [ ] Move nativebrowser, FormalShell, nothingctl, meshtop, messages, youtubemusic into `~/Developer/ecosystem/` once their panes are idle; `git worktree repair` after
- [ ] Repo-wide cargo config pointing target dir + sccache at the external drive; nix dev shell

## Phase 1: research (reports in `docs/research/`)
- [ ] GPUI: platforms (macOS, Linux Wayland/X11, Windows, web), multi-monitor refresh, a11y, IME, gestures, virtualization, blur, native view embedding (JaceThings tweet), gpui-component, gpui-kit.com
- [ ] macOS: AppKit + Liquid Glass hybrid, traffic light placement, toolbars, menus, haptics, CEF in-window
- [ ] Windows: WinUI 3 from Rust vs GPUI Windows, Mica, installer, auto-update
- [ ] Linux + Nix: matugen, Hyprland, home-manager module, config format choice
- [ ] Testing/ndshell: input simulation, frame recording, SIP, profiling, crash logs, all platforms
- [ ] Parity inventory: every feature of the `zig` branch (`docs/parity/zig.md`)
- [ ] App inventory: repeated patterns and handrolled components in messages, youtubemusic, nativebrowser, GPUI apps (`docs/parity/apps.md`)

## Phase 2: architecture
- [ ] `docs/plan/ARCHITECTURE.md`: renderer per platform, app API (GPUI-style vs reactive), crate layout, theming ("same look, native patterns"), config system, packaging
- [ ] Review pass by an adversarial agent before any code

## Phase 3: core framework
- [ ] Workspace skeleton, CI, nix flake, cargo config
- [ ] Windowing: resize, multi-window, multi-monitor refresh rate, window controls, drag regions, traffic light placement, Hyprland no-decorations
- [ ] Input: gestures (standard trackpad set), haptics, context menus and floating panels with no input leak-through
- [ ] Theming: platform look tokens, Liquid Glass (macOS), matugen (Linux), Mica (Windows)
- [ ] Virtualized lists/tables/grids (variable height, stick-to-bottom, no disappearing last rows)
- [ ] Component library (parity + handrolled app components + gpui-kit)
- [ ] Webview + CEF embedded as a true in-window surface
- [ ] System APIs parity (dialogs, clipboard, notifications, credentials, audio, tray, file assoc, URL schemes)
- [ ] Config system (TOML + home-manager module)
- [ ] Extension story (third-party crates, e.g. Discord RPC)
- [ ] Web target (if research says it is viable)

## Phase 4: tooling
- [ ] ndshell successor: cross-platform input simulation, frame recording, screenshots, SIP bypass on macOS, fast parallel runs without VMs
- [ ] Profiling built in (frame timing, per-monitor), flight-recorder logs with UI tree dumps and auto cleanup
- [ ] CLI: `nd new` bootstrap (CLAUDE.md, fun README, tests), `nd dev`, `nd build`, `nd package`
- [ ] Packaging: mac .app/dmg, Linux (nix, AppImage/flatpak?), branded Windows installer, auto-update
- [ ] Centralised project config file

## Phase 5: docs
- [ ] Docs site rebuilt (in the framework if viable, web target otherwise)
- [ ] Agent docs: CLAUDE.md template, do's and don'ts, testing guide
- [ ] Fresh screenshots per platform

## Phase 6: ports
- [ ] messages
- [ ] youtubemusic
- [ ] nativebrowser
- [ ] Prep for a Files/Finder-style file manager
