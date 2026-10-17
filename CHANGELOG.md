# Changelog

Releases are git tags on this repository. `nd upgrade` prints the entries between an app's tag
and the one it moves to.

## v0.6.0

The first release of the Rust rewrite. Tags up to v0.5.9 are the Zig framework, kept on branch
`zig`; nothing carries over from it.

- Apps are Rust crates on the `nd` crate. Windows draw through AppKit on macOS and through a
  patched GPUI elsewhere (Linux on Wayland and X11, Windows, and WebAssembly in a browser).
- Components: navigation, forms, media, tabs, panes, dialogs, virtualized lists, tables, grids and
  trees, Lucide and SF Symbols icons, the libghostty-vt terminal, web views (WKWebView, and CEF on
  macOS and Linux).
- Settings in TOML, theme tokens with live matugen reload, IPC, SQLite, plugins, and system APIs:
  dialogs, clipboard, notifications, keychain, tray, media keys and audio.
- Diagnostics: crash and hang bundles with the UI tree, `nd diagnose`, an automation socket and
  an MCP server.
- The `nd` CLI: `new`, `upgrade`, `check`, `test` (three test tiers), `build`, `serve`,
  `package` (macOS .app, .pkg and .dmg, Linux AppImage, Windows Setup.exe), `release` (signed
  delta updates from a self-hosted feed), `docs` and `doctor`.
- `nd new` depends on nd by git tag and points the GPUI patch block at the same tag, with nd's
  `Cargo.lock` as the starting lockfile. `nd upgrade` moves both to a newer tag.
- Known gaps: the Linux AppImage is untested on Linux, Setup.exe registers no file associations
  or URL schemes, and web views on Windows report a missing engine.
