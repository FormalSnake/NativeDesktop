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
| `gpui-polish` | user feedback for the GPUI look: e1504g resize lag, spacing, field contrast, OS fonts (SF exactly on macOS, desktop font on Linux), title in empty titlebar, matugen discovery, chrome settings cmd-comma panic | check its final report in TODO.md, finish, merge |
| `mac-polish` | user feedback for AppKit: titlebar/traffic-light alignment, rounded fields, centred status page, visible window tabs, Mail toolbar, title in empty titlebar | same |
| `icons-notes` | Lucide icons on GPUI/web, SF Symbols on macOS, empty icon slots collapse, notes as the flagship example | same |
| `port-gaps` | framework gaps from the messages port: rich text, animation and hover, role colours (SMS green), video and canvas, SVG, font override, focus by id, out-of-tree `nd new`, automation visibility | same; messages parity depends on it |
| `linux-webview` | Linux web engine = CEF off-screen via nd-cef (WPE is not in nixpkgs; WebKitGTK cannot embed in a GPUI Wayland window). One wip commit, never compiled | `scripts/linux-gate run 'cargo check -p nd-cef -p nd-webview -p nd-os-linux --tests'`, fix, then gates and an e1504g screenshot (TODO.md M8) |
| `m2-windows-2` | Windows: workspace builds in the g815 VM, OsHooks with Mica, drawn caption buttons, Snap Layouts; main merged in | nd-update apply on Windows, Setup.exe + signed update, the resize frame lag (fork patch), native run on real hardware |

Ports (separate repos under `~/Developer/ecosystem/`, never merged until a written parity checklist
is fully checked; then the nd branch becomes the app's main and the nix config switches to it):
- messages: branch `nd-port` (worktree `messages-nd`), macOS works on demo data, 63 of 89 parity items
  (`docs/nd-parity.md` there); waits on `port-gaps`; Linux leg not run.
- youtubemusic: branch `nd-port` (worktree `youtubemusic-nd`), macOS green incl. tier 3 test; the user's
  "library isn't fully loading" bug is fixed in the port (a dead session read as an empty library);
  Linux leg and Nix package not run.
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
