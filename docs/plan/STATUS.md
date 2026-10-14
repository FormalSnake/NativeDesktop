# Status, 2026-10-14 morning

`main` at dc2430f6: 32 crates, 14 examples, 172 commits since last evening. Merged lanes all
passed their own gates; `main` itself is being stabilized right now (see "Open").

## Done

| Milestone | State |
| --- | --- |
| M0 skeleton, first mac window | done: resize gate with real HID drag, traffic lights by coordinates |
| M1 diagnostics + harness | done: crash/hang bundles with UI tree, `nd diagnose`, locators, MCP, `nd new/check/test` |
| M2 backends + pacing | macOS and Linux done (Hyprland no controls, headless Hyprland test runner, per-monitor pacing 144/241 Hz on g815); Windows run on hardware 2026-10-14: per-monitor vblank pacing (240/144), resize gate all but 1 to 3 of 100 frames |
| M3 theme + mac chrome | done on macOS: tokens, matugen live reload (under 70 ms), `glass()`, sidebar/toolbar/inspector/settings as system components |
| M4 input + overlays | done: gestures with momentum and rubber band, haptics, focus rules, overlays that never leak input (proven over a live webview on macOS and Hyprland) |
| M5 virtualization | done: one list/table/grid/tree API, NSTableView natively, last-row bug class covered by tests; e1504g rerun pending |
| M6 components | navigation (your command palette, ported exactly), forms, media (player bar plays audio), tabs/panes/dialogs |
| M7 settings + system | done: TOML settings with Nix read-only mode and generated home-manager module, plugins + Discord RPC example, IPC, SQLite, dialogs/clipboard/notifications/keychain/tray/media keys/audio |
| M8 webviews | WKWebView and CEF (off-screen, part of the window) on macOS with full zig parity; Linux/Windows engines stubbed |
| M9 terminal | done on macOS and GPUI via libghostty-vt |
| M10 packaging + updates | macOS .app/.pkg/.dmg, signed delta updates from a self-hostable feed; Linux/Windows packages build but untested on their OS |
| M11 web | done: examples run in headless Chrome unchanged |

Also fixed along the way: dark-mode text colours (your e1504g report) plus a contrast/token audit
in `nd check`; your Nix direnv shells no longer leak Nix's SDK/xcrun/strip on macOS
(`~/.config/nix` 26032a31); daily disk cleanup timer on g815/e1504g (8c995a3a).

## Ecosystem move

All seven repos now live in `~/Developer/ecosystem/` (nativedesktop, nativebrowser, FormalShell,
nothingctl, meshtop, messages, youtubemusic). Worktree links repaired; messages' Find My launch
agent reinstalled at the new path and healthy. Old Zig code stays at `~/Developer/NativeDesktop`
on branch `zig`.

## Open

- `main` stabilization agent running: a few tests broke after the last merges, and the full suite
  takes 47 min; target under 10.
- Windows: hardware run done 2026-10-14 (`m2-windows-2`, TODO.md M2); open is Windows clippy, then the merge.
- e1504g: 100k-row scroll rerun pending (was offline).
- Not started: M12 docs site, agent docs and README template polish, ports of messages,
  youtubemusic and nativebrowser.

## Needs you

1. OK to push `~/.config/nix` (26032a31, 8c995a3a) so e1504g can pull its cleanup script.
2. Windows code signing choice, needed only at Windows packaging.
