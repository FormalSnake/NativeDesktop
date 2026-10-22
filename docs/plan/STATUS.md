# Status, 2026-10-22

v0.6.1 (e319443e, 2026-10-18) is the latest tag and the docs site runs it at native.formalsnake.dev.
`main` at `30e8e952`: 34 crates, 14 examples, 161 commits since 2026-10-18. Merged lanes passed their
own gates. The full test suite still takes 47 minutes.

## Per platform

| Platform | Works today |
| --- | --- |
| macOS | Real AppKit controls throughout. Stock NSToolbar, sidebar split item, inspector and settings as system components; `glass()`; the floating bottom accessory is plain Liquid Glass, content scrolls under it behind the system scroll edge effect; source list rows with artwork, size-following rows and animated inserts; mini switches in forms; window tabs; tab drag across lists, tile grids and windows; edge-revealed sidebar; command palette with fade; WKWebView and CEF (Chrome-style pages, extensions, Web Store installs, request filter, private sessions, print) |
| Linux | GPUI with a libadwaita skin: palette, radii, motion, controls, popovers, toast, banner, alerts, backdrop title row, raised shadow and undershoot at scroll edges, a header bar per pane, neutral window controls with the GNOME accent. Matugen live recolour, headless Hyprland runner, CEF off-screen with extension popups in an nd view, GStreamer video, built-in emoji picker, tray icons, print through the desktop portal, apps that outlive their windows |
| Windows | GPUI with a WinUI 3 skin: Mica, drawn caption buttons with Snap Layouts, NavigationView rows, Segoe Fluent glyphs, ContentDialog alerts and about panel, access keys with key tips, deactivated title row, Fluent motion, frame and Mica following the app theme. Media Foundation video, system emoji panel, per-monitor vblank pacing (240/144 Hz). Workspace tests and clippy pass in the VM. Resize gate fails only the frame check, 1 to 3 of 100 frames one drag step behind |
| Web | Wasm GPUI backend with the plain GPUI look; every example runs in headless Chrome. The `webview()` iframe is not built |

## Per crate group

- Core: `nd`, `nd-core`, `nd-ui` (apps can run with no window open and quit on their own), `nd-theme`, `nd-components` (scaffold, shelves, bottom accessory, conditional builders), `nd-virtual`, `nd-gesture`.
- Backends: `nd-backend-appkit`, `nd-backend-gpui`, `nd-os-linux`, `nd-os-windows`, `nd-os-web`, `nd-gpui` (fork patches up to 0010).
- Content: `nd-webview`, `nd-cef`, `nd-terminal`, `nd-icons`.
- System and app services: `nd-system`, `nd-config`, `nd-settings`, `nd-data`, `nd-ipc`, `nd-plugin`, `nd-update`.
- Tooling: `nd-cli`, `nd-diag` (a panic off the main thread exits 101 without running exit handlers), `nd-automation`, `nd-test`, `nd-mcp`, `ndshell` (shot command, libadwaita reference captures, Windows capture over the named pipe), `nd-docs`.

## Apps

| App | State |
| --- | --- |
| Examples (14) | Run on all three native platforms and the web. Docs captures: Linux and Windows 13/13, macOS recaptured for v0.6.1 |
| FormalMusic (youtubemusic port) | Runs on macOS, g815 and the Windows VM; deployed on g815 and the macbook with nd `30e8e952`. Pill player bar, lyrics depth of field, hero and presentation motion, Kopuz daemon underneath. Linux lacks the expanded player open animation and a per-page header title |
| Messages | Runs on macOS and Linux (Linux CSD and app id open). Idle until the next nd bump |
| nativebrowser | Skeleton (`crates/lynk`: sidebar and one CEF page) on its `nd-port` branch; the nd lanes it waited on (Chrome-style pages, extensions, request filter, private windows, tab drag, sidebar reveal, print) are merged |

## Milestones

| Milestone | State |
| --- | --- |
| M0 to M1 | done |
| M2 backends + pacing | macOS and Linux done; Windows on hardware and in the VM, resize frame check at 1 to 3 of 100 |
| M3 theme + chrome | done on macOS, Linux (libadwaita) and Windows (Fluent) |
| M4 input + overlays | done on macOS and GPUI; WebView2 host not written |
| M5 virtualization | done; e1504g 100k rows p95 17.0 ms at 60 Hz |
| M6 components | done: navigation, forms, media, tabs, panes, dialogs |
| M7 settings + system | done |
| M8 webviews | WKWebView and CEF on macOS and Linux; WebView2 and WPE not written |
| M9 terminal + CEF | done on macOS and GPUI; Linux and Windows terminal runs open |
| M10 packaging + updates | macOS done; Linux and Windows packages build but untested on their OS |
| M11 web | done |
| M12 docs | site live; push-triggered rebuilds wait on the Cloudflare GitHub app (see below) |
| M13 to M15 ports | messages and FormalMusic running; nativebrowser at skeleton |

## Open

- Open lanes: FormalMusic Linux motion, now playing, Linux title; docs-captures-2; Kopuz MPRIS identity; windows-hig Alt and window menu checks (HANDOFF).
- Test suite: 47 minutes, target under 10.
- Linux CI fails only the headless Chrome embed render check.
- FormalMusic automation tree on g815 lists 2 nodes before the library loads, and the installed build lacks the `capture` feature.
- Not started: OS-release diff job, NSScrollView-driven scroll for terminal and CEF surfaces, cross-label text selection on GPUI.

## Needs you

1. A Kopuz release after the `mpris-app-identity` PR merges (approved 2026-10-22), so the flake moves past v0.22.0.
2. Move the nd repo's `build-dir` in `.cargo/config.toml` (still on `/Volumes/Kyan Drive`) to the ecosystem-level config?
3. Replies on upstream GPUI PRs #65728, #65729, #65730.
4. Windows code signing choice, needed only at Windows packaging.
