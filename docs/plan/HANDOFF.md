# Handoff, 2026-10-14

Work paused on the user's request to save Claude Code usage (weekly limit resets Tuesday
2026-10-20). Resume from here. Read first: `docs/plan/PROMPT.md` (verbatim request plus every
dated decision), `docs/plan/STATUS.md`, `docs/plan/TODO.md`, `docs/dev/setup.md` (machine rules).

## Where things are

`main` on GitHub (FormalSnake/NativeDesktop) is the Rust rewrite; the old Zig framework is branch
`zig`. M0 to M12 are built and merged; the full suite runs in under 6 minutes on the macbook. Open
lane branches (local worktrees under `~/Developer/ecosystem/nativedesktop-wt-*`, pushed for
visibility, not merged unless noted):

| Branch | State | Next step |
| --- | --- | --- |
| `gpui-polish` (ae5e4de1) | done: e1504g resize lag fixed (fork patch 0006, 30-40 to 58-60 fps), FormalShell theme.json read for matugen, chrome settings panic, field fill, libadwaita spacing, title in empty titlebar, OS fonts (SF exact on macOS) | re-record GPUI goldens on g815 (`ND_UPDATE_GOLDENS=1` on nd-backend-gpui forms, nav, media) and macOS, check by eye, test for patch 0006, make the resize gate catch slow-GPU lag, merge |
| `mac-polish` (21f5a387) | done: titlebar centre line, rounded multi-line field, centred status page, cmd-T crash fixed with tab bar at 2+ tabs, Mail inspector toggle trailing, title in empty titlebar; overrides goldens re-recorded | re-record gallery status-page and forms goldens, dark recaptures, check hello's title, clippy/fmt, merge |
| `port-gaps` (f968d503, one wip commit) | rich text, animation/hover, role fills, GPUI canvas, SVG, font override, focus by id, automation rows written, mostly never compiled | `cargo test --no-run -p nd-components -p nd-backend-appkit`, fix; then video, AppKit canvas, out-of-tree `nd new` |
| `linux-webview` (6349761d, wip) | Linux engine = CEF off-screen via nd-cef (no WPE in nixpkgs; WebKitGTK cannot embed in GPUI on Wayland), never compiled | `scripts/linux-gate run 'cargo check -p nd-cef -p nd-webview -p nd-os-linux --tests'`, fix, gates, e1504g screenshot (TODO.md M8) |
| `m2-windows-2` (275dd12d) | VM builds and tier 1+2, OsHooks with Mica, caption buttons, Snap Layouts; native run script ready, never run. UNCOMMITTED in its worktree `nativedesktop-wt-windows2`: nd-update Windows apply fix, packaging + signed-update test, resize fix copy of GPUI's Windows crate (also uncommitted in g815 `~/.cache/nd-m2-windows/zed`) | commit those after building them in the VM, move the resize fix into `scripts/gpui-fork`, Setup.exe + update in the VM, native hardware run (needs the user's PIN) |
| `port-ytm-fixes` (402a0efa) | Wayland app id from `[app] id`, `nd new --nd-path` writes the GPUI patch block; untested | test, merge |

Merged into main this session: icons on every backend (`nd-icons`, Lucide + SF Symbols), the notes
example, docs app (35 pages, macOS captures), the messages-port fixes, the e1504g scroll budget.

Ports (separate repos under `~/Developer/ecosystem/`, never merged until a written parity checklist
is fully checked; then the nd branch becomes the app's main and the nix config switches to it):
- messages: branch `nd-port` (worktree `messages-nd`), macOS works on demo data, 63 of 89 parity items
  (`docs/nd-parity.md` there); waits on `port-gaps`; Linux leg not run.
- youtubemusic: branch `nd-port` (88728dd, worktree `youtubemusic-nd`), macOS green incl. tier 3 test;
  the user's "library isn't fully loading" bug: their YouTube session cookies are dead; the port now
  reads the sign-in answer as signed out and prompts. Linux leg, Nix package and switch not run; nix
  edits wait on branch `formalmusic-nd` (worktree `~/.config/nix-wt-formalmusic`); framework gaps in
  its `docs/port-parity.md` (dispatch buttons, media icons, drag reorder, container hover, menu button).
- nativebrowser: not started; needs the Linux engine and Chrome-style extensions (M15).

## Machines

- macbook: overloaded host for the user's services (BlueBubbles). The repo's rustc wrapper builds at
  low priority, the dev shell caps jobs at 4. Only macOS work runs here.
- g815: NixOS, small root disk. Every build goes through `scripts/linux-gate` (one build at a time via
  `~/.cache/nd-build/build.lock`, shared target capped at 25 GB, no incremental). Check `df -h /`.
  Headless Hyprland only; never touch the user's displays or power settings.
- e1504g: never build on it. Build on g815, copy binaries, run, delete.
- Windows: the VM on g815 boots the real Windows disk behind a per-session overlay (nothing reaches
  the real install). Native Windows runs need someone at the keyboard for the PIN.

## Pending with the user

- iGPU passthrough for the Windows VM plus a TEMPORARY SDDM autologin: branch `g815-igd` in
  `~/.config/nix` (worktree `~/.config/nix-igd`), built, not staged. After the formalshell session's
  combined rebuild is done: rebase onto main (drop 5b0bef66, already on main), `nixos-rebuild boot`,
  user reboots; then verify passthrough, VM, real Windows partition unchanged, and turn autologin off
  (`systems/g815/temp-autologin.nix`) with a rebuild and push.
- Windows code signing choice (only needed for Windows packaging).

## Rules that bit us (keep them)

No `rm`/`mv` on variable paths; never `pkill -f` by pattern; never git reset/checkout/restore/stash;
one heavy macbook build at a time; disk checks before builds on both the cache drive and g815;
`just clean-merged` after merging lanes; every merge pushed to GitHub main; fix bugs found, never
note-and-move-on.

## Scheduled resume

A launchd job (`~/Library/LaunchAgents/dev.formalsnake.nd-resume.plist`, removes itself after it
runs) fires 2026-10-20 14:00, or at the next wake after that: `scripts/resume-in-herdr` starts a fresh
`claude` session in Herdr pane `w1F:p3` (a new workspace if that pane is gone) with
`docs/plan/RESUME-PROMPT.md` as its first prompt. Log: `~/Library/Logs/nd-resume.log`.
