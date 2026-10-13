# App inventory: what the ecosystem hand-rolled

Read-only survey, 2026-10-13. Paths are relative to `~/Developer`. Line counts are
`wc -l` at survey time; several repos had other agents editing.

## 0. Corrections to the brief

- GPUI apps: only `messages` and `youtubemusic` (both `gpui-kit`/`gpui-base` 0.6.6 on
  `gpui-pre` 0.3.6). `youtubemusic` is the Rust app called **FormalMusic**
  (binary `formalmusic` plus daemon `formalmusicd`).
- `~/Developer/formalmusic` is the **old Electron** FormalMusic (README is the electron-vite
  template, 34 lines). It is not a README style source; `youtubemusic/README.md` is.
- `meshtop` is a ratatui TUI, `nothingctl` is a clap CLI (no UI), `kopuz` is Dioxus 0.7,
  `FormalShell` is a custom Wayland shell (smithay-client-toolkit + own draw code),
  `formalterm` is Electrobun/React, `formallauncher` is Electron/React/Hono.
  None of those five use GPUI.
- FormalShell has ~35 sibling worktrees (`FormalShell-m52` ... `-m71`, `-earbuds`, ...). Not
  surveyed beyond `FormalShell/`.

## 1. Per repo

### messages (GPUI, Rust, 31k lines) - iMessage client for Linux/Windows
Stack: GPUI (gpui-kit), tokio, BlueBubbles over HTTP + hand-written socket.io v4 client
(`crates/core/src/bluebubbles/socket.rs`), Bun/TS mac agent (`apps/mac-agent`), Nix flake + HM module.
Hand-rolled UI (all `messages/crates/desktop/src/`):
- Virtualized bottom-aligned thread: `thread.rs` (`RowList`), `thread_rows.rs` (date separators, runs)
- Message bubble with tails, tapbacks, run grouping: `bubble.rs`; selectable text: `selectable.rs`
- Sidebar virtualized `uniform_list`, pinned strip, search: `sidebar.rs`, `sidebar_row.rs`, `search_results.rs`
- Composer (rich text, attachments staging, send-later): `composer.rs`, `scheduled.rs`
- Attachment grid, video poster, link embeds: `attachments.rs`; video player `video.rs`
- Lightbox (image/video carousel): `lightbox.rs`
- GIF picker + animated GIF clock/LRU: `gif_picker.rs`, `gif.rs`; sized stills cache `stills.rs`
- Context menus, tapback picker: `menus.rs`; confirm dialog `confirm.rs`; toast `toast.rs`
- Ctrl+K switcher (command palette): `switcher.rs`; new-chat recipients picker `new_chat.rs`
- Details/info panel (sliding, docked): `details.rs`; Find My map card `location.rs`
- Reply thread pane `reply_thread.rs`; FaceTime banner `facetime.rs`
- Trackpad two-finger swipe-to-reply: `swipe.rs`
- Primitives (Avatar, Button, IconButton, Chip, Divider, SectionLabel): `primitives.rs`; icon map `icons.rs`
- Title-bar drag strip + caption reserve: `chrome.rs`
- Theme tokens + live matugen reload: `theme.rs`, `live_theme.rs`; motion helpers/presence `motion.rs`
- Fonts and colour emoji workaround on Linux: `fonts.rs`, `emoji_font.rs`
- Single instance: `single_instance.rs`; startup trace: `trace.rs`; store-to-view bridge: `bridge.rs`
- Test harness over GPUI test platform: `app_tests.rs`

### youtubemusic / FormalMusic (GPUI, Rust, 47k lines) - YouTube Music client
Stack: GPUI + a separate `formalmusicd` daemon (InnerTube, yt-dlp, rodio-style engine, MPRIS,
tray, scrobbling) speaking JSON lines on a unix socket / Windows named pipe. Nix flake + HM module.
Hand-rolled UI (`youtubemusic/crates/desktop/src/`):
- Player bar (transport, seek, volume): `player_bar.rs`; seek clock interpolation `clock.rs`
- Expanded player + cover with animated Apple Music video: `now_playing.rs`, `cover_video.rs`, `music_video.rs`
- Synced lyrics view (per-word fill, glow, follow/resync, wheel takeover): `lyrics.rs` (2193 lines, spec in `docs/lyrics.md`)
- Generic page renderer (header, chips, shelves) on a virtualized `list`: `page.rs`
- Shelves/carousels/cards/quick-picks with horizontal ScrollHandle: `shelves.rs`
- Page headers in three shapes: `header.rs`
- Sidebar (nav + playlists): `sidebar.rs`; top bar (back/forward, search with suggestions): `topbar.rs`
- Settings dialog (playback, EQ, privacy): `settings.rs` (1106 lines)
- Sign-in flow (browser profile picker, cookie paste): `signin.rs`
- Dialogs: `edit_playlist.rs`, `new_playlist.rs`; shortcuts overlay (`?`): `shortcuts.rs`
- Context menus, item actions: `menus.rs`, `actions.rs`; toast `toast.rs`; primitives `primitives.rs`
- Artwork cache decoded at display size: `art.rs`; fonts `fonts.rs`; theme `theme.rs`, `live_theme.rs`, motion `motion.rs`
- Daemon-side shell integration: `crates/daemon/src/mpris.rs`, `tray.rs` (ksni), `shell.rs` (Windows SMTC + notification icon)
- Equalizer: `crates/player/src/equalizer.rs` (UI is inside `settings.rs`)

### nativebrowser / Lynk Browser (NativeDesktop 0.4.56, TS/Solid, 11.6k lines) - Arc-style browser on CEF
Stack: NativeDesktop Zig host + Bun/Solid child, CEF 151 for pages, AppKit on macOS, GTK4 on Linux.
Hand-rolled UI (`nativebrowser/src/`):
- Window shell with sidebar/compact layouts, omnibox command bar, split panes, site panel, extensions panel: `BrowserWindow.tsx` (2334 lines)
- Vertical tab sidebar with pinned tabs and groups: `Sidebar.tsx`; compact tab strip `CompactTabs.tsx`
- History / bookmarks / downloads panels with date-grouped lists: `Panels.tsx`, `Downloads.tsx`
- Zoom control popover: `ZoomControl.tsx`; popup-blocked bar `PopupBlocked.tsx`; private window `PrivateWindow.tsx`; popup window `PopupWindow.tsx`
- App root, settings window, session restore, multi-window registry: `App.tsx` (2535 lines)
- Non-UI libs the framework could own: `lib/omnibox.ts` (ranking), `lib/session.ts`, `lib/settings.ts`, `lib/downloads.ts`, `lib/permissions.ts`, `lib/adblock.ts`, `lib/favicons.ts`, `lib/webstore.ts`

### FormalShell (Rust, 139k lines) - Hyprland desktop shell
Stack: one Rust binary, smithay-client-toolkit + own software/GPU draw (`formalshell-rs/src/ui/draw.rs`, 1227 lines),
`fs-*` service crates (audio, network, bluez, upower, tray, notifd, mpris, media), plugins as child processes with JSON lines,
Nix flake + HM module + NixOS VM tests. README is the style reference.
Hand-rolled UI (`FormalShell/crates/formalshell-rs/src/`):
- Launcher / command palette with modes (apps, clipboard, calc, emoji, dmenu-style select): `surfaces/launcher.rs` (2367), `wayland/launcher.rs`
- Bar with cells, slots, chevron overflow, workspaces with live previews: `surfaces/bar/`, `fs-chrome/src/bar/layout.rs`
- Panels (audio, network, bluetooth, calendar, weather, power, display, monitor, media, iphone, earbuds): `surfaces/panel/`
- Toasts and notification centre: `surfaces/toasts.rs`, `surfaces/panel/center.rs`; tooltip `surfaces/tooltip.rs`; OSD `surfaces/osd.rs`
- Tray menu: `surfaces/tray_menu.rs`; popup/modal: `surfaces/popup.rs`, `surfaces/modal.rs`; card `surfaces/card.rs`
- Alt+Tab switcher with thumbnails: `surfaces/switcher/`
- Screenshot/record/OCR picker: `surfaces/capture/`; lock + greeter + screensaver: `surfaces/lock.rs`, `wayland/lock.rs`, `wayland/screensaver.rs`
- Globe/atlas, audio visualizer (31 styles): `surfaces/atlas/`, `fs-media/src/visualizer/styles.rs` (2518)
- Synced lyrics: `fs-media/src/lyrics.rs` (1586) - the spec `youtubemusic` lyrics.rs ports
- Theme engine (3 presets, wallpaper palette, bar paint): `fs-theme/src/style.rs`, `fs-theme/src/barpaint.rs`
- Plugin host: `fs-chrome/src/plugins.rs`

### kopuz (Dioxus 0.7, Rust, 132k lines) - music player (third-party fork, not authored from scratch)
Stack: Dioxus (webview), sqlx/SQLite, `kopuzd` daemon + `kopuz-ctl`, proto crate, Bazel and Nix and Android.
UI components (`kopuz/crates/components/src/`): track list/row/virtual scroll (`track/`, `common/virtual_scroll.rs`),
queue with drag (`queue/`), bottom bar and fullscreen player (`layout/`, `playback/`), equalizer (`settings/equalizer.rs`),
sort control, schema-driven settings form (`forms/schema_form.rs`), source switcher, selection bar, modals.
Config layering worth copying: `kopuz/crates/config/src/store.rs` (blob, `settings.toml`, `settings.d/*.toml`, `KOPUZ_CONFIG_*` env).

### meshtop (ratatui TUI, 2.4k lines)
Per-host cards, topology map, agents list, ssh JSON-line agent. `meshtop/src/ui/{overview,map,agents,detail}.rs`,
config `meshtop/src/config.rs` (TOML). Needs from the framework: nothing visual. It needs the config + HM module + flake pattern.

### nothingctl (clap CLI, 3.6k lines)
Bluetooth RFCOMM client for Nothing/CMF earbuds (`status`, `set`, `watch`). `nothingctl/src/protocol/`, `transport/linux.rs`.
FormalShell already vendors the idea (`fs-devices/src/earbuds/`). Framework relevance: a `watch` JSON-lines mode and a
Bluetooth service crate, plus the CLI/flake template.

### formalterm (Electrobun, TS)
Terminal in ghostty-web, PTY over RPC; hiddenInset titlebar with `-webkit-app-region: drag` (`formalterm/src/mainview/App.tsx`).
Needs: terminal widget (the Zig framework already shipped `<terminal>`), window chrome/drag area. 3 files of real code.

### formallauncher (Electron + Hono + Postgres, TS)
Minecraft launcher: instances, Modrinth browse/install, MS device-code login, skin editor, sync server.
`formallauncher/apps/desktop` (ShadCN new-york), `apps/server`. Needs: card grid, search results list, tabs, dialogs,
progress bars, account switcher. Not on the port list.

## 2. Repeated patterns the framework should own

Evidence: 2 paths max, then a count of repos with the pattern (of the 9 surveyed).

| Pattern | Evidence | Repos |
|---|---|---|
| matugen theme.json live reload | `messages/crates/desktop/src/live_theme.rs`, `youtubemusic/crates/desktop/src/live_theme.rs` (1 s mtime poll, swaps a `Global` palette, `refresh_windows`); `youtubemusic/nix/hm-module.nix` writes it | 5 (messages, youtubemusic, FormalShell, kopuz, +HM modules) |
| Palette/spacing/radius/type-scale tokens + `Theme::get(cx)` | `messages/crates/desktop/src/theme.rs` (304), `youtubemusic/crates/desktop/src/theme.rs` (321) near-duplicates | 3 |
| Config files: JSON today, TOML wanted | `messages/crates/core/src/config.rs` (JSON, keeps unknown keys), `meshtop/src/config.rs` (TOML), layered `kopuz/crates/config/src/store.rs` | 6 |
| Env override per app (`MESSAGES_FONT`, `FORMALMUSIC_SOCKET`, `KOPUZ_CONFIG_*`) | `messages/crates/desktop/src/theme.rs:16`, `youtubemusic/README.md` Configuration | 4 |
| Nix flake + package.nix + hm-module.nix | `messages/nix/hm-module.nix`, `youtubemusic/nix/hm-module.nix` (identical shape, `programs.<app>.enable/theme`) | 5 flakes with HM modules |
| Packaging: desktop entry, icon, .ico, PowerShell installer | `youtubemusic/packaging/windows/install.ps1`, `messages/packaging/linux/dev.formalsnake.messages.desktop` | 6 |
| App id convention `dev.formalsnake.<app>` | `messages/packaging/linux/dev.formalsnake.messages.desktop`, `youtubemusic/PLAN.md` | 3 |
| Auto-update: absent in every GPUI app | only distro/Nix; `youtubemusic` replaces it with a weekly Claude maintenance run (`youtubemusic/maintenance/weekly.md`) | 0 real |
| Single-instance lock + raise existing window | `messages/crates/desktop/src/single_instance.rs` (85), `youtubemusic/crates/desktop/src/single_instance.rs` (identical) | 3 |
| IPC: JSON lines over unix socket / named pipe | `youtubemusic/crates/api/src/local.rs`, `kopuz/crates/proto/src/pipe.rs`; FormalShell `formalshell-ipc` | 4 |
| Daemon/UI split so media survives window close | `youtubemusic/crates/daemon/`, `kopuz/crates/kopuzd/` | 2 |
| MPRIS (Linux) and SMTC (Windows) media keys | `youtubemusic/crates/daemon/src/mpris.rs`, `youtubemusic/crates/daemon/src/shell.rs` (SMTC); also `kopuz/crates/player/src/systemint/linux.rs` | 3 |
| Tray (SNI via ksni, Windows notification icon) | `youtubemusic/crates/daemon/src/tray.rs`, `FormalShell/crates/fs-tray/src/` (watcher side) | 4 |
| Notifications shelled out per OS (notify-send, osascript, PowerShell toast) | `messages/crates/core/src/notify.rs` (210) | 4 |
| Persistence: `state.json` cache painted before network, XDG state dir | `messages/docs/rust-architecture.md` (cold start < 150 ms), `youtubemusic/crates/core/src/cache.rs` | 5 |
| SQLite | `kopuz/crates/db` (sqlx), `nativebrowser` via `@nativedesktop/data` | 2 |
| Logging: tracing + ad hoc `*_TRACE=1` stderr timings, no file rotation | `messages/crates/desktop/src/trace.rs`, `youtubemusic/crates/desktop/src/trace.rs`; kopuz has `tracing-appender` | 4 |
| Window chrome: drag strip, caption reserve, `occlude()` rule | `messages/crates/desktop/src/chrome.rs`, `youtubemusic/crates/desktop/src/chrome.rs` | 3 |
| Fonts: bundled Noto Sans on Linux, SF Pro on macOS, colour emoji hack | `messages/crates/desktop/src/fonts.rs`, `messages/crates/desktop/src/emoji_font.rs` | 2 |
| Image/art cache decoded at display size, LRU, `IN_USE` window | `messages/crates/desktop/src/stills.rs`, `youtubemusic/crates/desktop/src/art.rs` | 2 |
| Store-event to view-notify bridge (topic watch) | `messages/crates/desktop/src/bridge.rs`, `youtubemusic/crates/desktop/src/bridge.rs` | 2 |
| Context menu data model (`MenuItem`) + overlay stack in root | `messages/crates/desktop/src/menus.rs`, `youtubemusic/crates/desktop/src/menus.rs` | 3 |
| Demo mode (`*_DEMO=1` fixtures) for screenshots/tests | `messages/crates/core/src/demo.rs` (1154), `youtubemusic/crates/core/src/demo.rs` (2198) | 2 |
| Presence/motion helpers (fade, cubic bezier, reduce motion) | `messages/crates/desktop/src/motion.rs`, `youtubemusic/crates/desktop/src/motion.rs` (identical doc) | 2 |
| Headless Linux screenshot rig, sway/Hyprland smoke | `messages/scripts/linux-headless.sh`, `FormalShell/dev/smoke.sh` | 3 |
| Plugin/extension model (child process, JSON lines) | `FormalShell/crates/fs-chrome/src/plugins.rs`; Discord RPC crate `kopuz/crates/discord-presence/src/lib.rs` | 2 |

## 3. Virtualization and scroll pain

How messages did it (Rust): GPUI `list(ListState, render_item)` with `ListAlignment::Bottom` and
`FollowMode::Tail` (`messages/crates/desktop/src/thread.rs:119-130`). On top of that, three hand-built layers:
1. `RowList::apply` diffs old/new slots by key (common prefix and suffix), then calls `ListState::splice`
   on the changed middle and `remeasure_items` per changed row (`thread.rs:151-190`). `reset` drops everything for a chat switch.
2. Row entities (`MessageRow`) live in a `HashMap<key, Entity>` only for rows the list actually lays out;
   a settled row is wrapped in `AnyView::cached` at its last measured size (`thread.rs:243-251`).
3. `RowLayout { measured, given, stale }` (`bubble.rs:351-358`): a canvas in each row compares the height the
   cached box was given against the real content height and, if off by 0.5 px, sets `stale` and requests an animation
   frame (`bubble.rs:998-1008`). A `width_probe` canvas feeds the list width in before rows lay out (`thread.rs:213-218`).
Infinite scroll up: `OLDER_WITHIN_ROWS = 3` triggers a page load (`thread.rs:811`). `JUMP_PAGES = 12` and
`ListOffset { item_ix, offset_in_item: -96 }` implement jump-to-message.
Sidebar uses `uniform_list` (`sidebar.rs:459`). `youtubemusic` page.rs uses `ListState` with `ListAlignment::Top`
(`page.rs:139`), `scroll_to_end` after playlist actions (`app.rs:955`).

The original TS client used a framework `<virtual-list estimatedItemHeight={44} overdraw={400} followTail>`
(removed in `messages` commit 4cbde88; read with `git show 4cbde88^:apps/desktop/src/ui/thread.tsx:830-846`).
`rust-parity.md:494` marks it "Virtualized, bottom-aligned, follows the tail".

"Last rows disappearing": fixed in `messages` (user-confirmed 2026-10-13); no commit message names it, so the
working list stack above is the reference implementation to port. The framework list must make each of the mechanisms
below structural so no app has to rediscover them. The failure class:
- `messages/CLAUDE.md` "GPUI rules": a cached view whose root has only `flex_grow` **collapses to zero height**
  (needs `flex_basis(0)` + `h_full`); that makes tail rows vanish.
- Same file: overlays must be `<anchored deferred>` or they paint under the virtual list (TS era, `4cbde88^:CLAUDE.md:187-189`).
- `04ed20f` "size image boxes from the file header so rows never overlap": row height wrong before the image loads, so later
  rows overlap or clip. `RowLayout.stale` exists for the same reason: a row that grows after caching clips the tail.
- GIF/video rows changing height after measure force `remeasure_items` (`thread.rs:186`).
Requirements this implies for the framework list: (a) variable heights that change after first layout (images, GIFs, link
embeds, expanding replies), (b) bottom anchoring with stable offset when older rows prepend, (c) tail-follow that stops when
the user scrolls up, (d) jump-to-item with offset, (e) key-based diff so an update never resets scroll, (f) fully measured total
height so the last rows are always reachable, (g) cached/clean rows so one row's animation repaints only that row.
Other scroll pain: details panel wheel handed to the thread (`547caeb`, `7d950b7`, `64cd5b3`), nested scrollers (TS rule: one
scroller per column), lyrics wheel takeover with follow/resync (`youtubemusic/docs/lyrics.md:399`), horizontal carousels
need per-section `ScrollHandle` (`youtubemusic/crates/desktop/src/page.rs:91`), `<table>`/`<listview>` in Zig capped at 10 rows
(NativeDesktop CLAUDE.md).

## 4. README style (messages, FormalMusic, FormalShell)

Note: `formalmusic/` (Electron) is a template README. Using `youtubemusic/README.md` for "formalmusic".
- Opens with a centered 160 px icon (`<p align=center><img ...>`), `# Name`, then ONE plain sentence: "iMessage on Linux. A native Rust client (GPUI) that talks to ...". FormalShell opens with an ANSI-shadow ASCII banner and shields.io badges (`for-the-badge`, `labelColor=161616`, one accent colour).
- Second paragraph: how it works in 2-3 sentences, then a boxed ASCII architecture diagram (two or three boxes, arrows labelled with the protocol).
- A concrete performance/behaviour boast in a plain sentence: "It idles at 0% CPU." "Threads update live."
- Hero screenshot, then a 2-column table of screenshots (`| | |` header trick). FormalShell uses GIFs and a themes table.
- `## Features`: short bullets or a capability matrix table with yes/no columns (messages: SIP on / Private API).
- Honesty line: "A widget with nothing behind it says so (`NO ADAPTER`) instead of inventing data." Status is stated ("Pre-alpha. SWITCHOVER.md tracks what has run on real hardware").
- `## Install` always: `### Nix` (run command + Home Manager snippet with `programs.<app>.enable`), `### Other Linux` (deps list + `git clone`/`cargo build --release`), `### Windows` (install.ps1 and where files land, pipe names, log path).
- Then task sections: Signing in / Mac setup, Configuration (file path, tiny JSON sample, env vars list, a `socat` one-liner for the socket), Keyboard (two-column table, "`Cmd` on macOS."), Plugins/IPC for FormalShell.
- `## Development`: 4-5 commands in one fenced block with trailing comments aligned (`# recorded responses`), then one paragraph naming each crate in a sentence, and a pointer to `CLAUDE.md`/`PLAN.md`.
- Ends with `## License`: "[MIT](LICENSE). The Noto Sans faces in `packaging/fonts` are under the [OFL]. Not affiliated with ...".
- Tone: lowercase-confident, second person only in setup, no marketing words, no emoji, no em dashes, numbers over adjectives (31 visualizer styles, 100 rows kept, under 150 ms). Fun comes from specifics and dry asides, not exclamation marks. Whole README 175-260 lines.
- nativebrowser's README (Lynk) is the NativeDesktop-flavoured variant: two icons side by side (macOS vs Linux), badge row including `nativedesktop-0.4.56`.

## 5. Ranked components the new framework should ship

Rank = number of apps using it x how expensive it was to hand-roll. (S/M/L = effort in rewrite.)

1. **Virtual list** (variable height, bottom anchor, tail follow, prepend-stable, jump-to, keyed diff, cached rows) - messages thread/sidebar, youtubemusic page, nativebrowser history, kopuz tracks, FormalShell launcher. L
2. **Context menu / popover / floating panel layer** with input capture (no leak-through) - every app; includes tapback picker, tray menu, popup. L
3. **Command palette / switcher** (fuzzy rank, modes, kbd nav) - messages `switcher.rs`, nativebrowser omnibox, FormalShell `launcher.rs`. M
4. **Theme system**: tokens + matugen watcher + light/dark + accent; `Theme::get(cx)` - 5 apps. M
5. **Window chrome**: drag area, caption buttons, traffic-light placement, inset title bar - 4 apps. L
6. **Toast / OSD / banner** - messages, youtubemusic, FormalShell, nativebrowser PopupBlocked. S
7. **Primitives**: Button, IconButton, Chip, Avatar, Divider, SectionLabel, Switch, Slider, Select, TextField, SearchField with suggestions. M
8. **Dialog / sheet / confirm / form** (settings dialog, edit playlist, schema-driven settings form `kopuz/.../schema_form.rs`). M
9. **Sidebar / source list** with sections, badges, pinned strip, drag reorder, trailing actions - messages, youtubemusic, nativebrowser, kopuz. M
10. **Tabs / tab strip** (vertical + compact, pinned, drag out) + split panes - nativebrowser, kopuz fullscreen tabs. L
11. **Image/artwork pipeline**: decode at display size, LRU, animated GIF/video frame clock, lightbox. M
12. **Media transport kit**: player bar, seek bar with clock interpolation, volume, queue with drag reorder, MPRIS/SMTC/MediaRemote service. L
13. **Synced lyrics view** (per-word fill, follow/resync) - youtubemusic, FormalShell, kopuz. M
14. **Carousel / shelf / card grid** with horizontal scroll handle - youtubemusic, kopuz, formallauncher. S
15. **Rich text composer** (multiline, attachment chips, mentions, send-later). L
16. **Details/inspector panel** (docked sliding, own scroll) - messages, nativebrowser site panel. S
17. **Gestures**: two-finger swipe, pinch, long press, haptics - messages `swipe.rs` only, but brief asks for standard. M
18. **Tray/notification/single-instance/IPC service crate**, shared by all daemons. M
19. **Config crate**: layered TOML + `settings.d` + env + HM module generator (`kopuz/crates/config/src/store.rs` as model). M
20. **Table / tree / data grid** - kopuz tracks, FormalShell monitor, meshtop cards (TUI). M
21. **Charts / spark lines / visualizer** (EQ graph, system monitor, 31 styles) - FormalShell, kopuz, meshtop. M
22. **Map / globe** embed (Find My, atlas) - messages `location.rs`, FormalShell `atlas/`. L, low priority
23. **Terminal widget** (formalterm, FormalShell) - already in the zig branch; port. L
24. **Embedded CEF webview** hosted as a true child (nativebrowser). L, tracked separately in the brief.
