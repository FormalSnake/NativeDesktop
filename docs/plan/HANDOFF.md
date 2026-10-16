# Handoff, 2026-10-16

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
| `gpui-polish` | goldens re-recorded (Linux and macOS), patches renumbered after main's (Windows 0005 to 0007, apple 0008, swapchain 0009), weak-GPU resize gate added; patch 0009 leaves a 4 to 8 px background strip at the right edge in 115 of 127 drag frames on g815 (passes with `ND_DEBUG_SWAPCHAIN=eager`) | an agent is root-causing the strip (wp_viewport crop vs committed buffer in gpui_linux window.rs); merge when `real_resize_drag` passes |
| `linux-webview` | being compiled and gated on g815 | CEF off-screen gates in headless Hyprland, e1504g screenshot, merge |
| `messages` `nd-port` | 63 of 89 parity items; port-gaps landed | an agent is closing parity items on macOS; framework gaps go to nd branch `msg-gaps` |

Merged into main on 2026-10-16: `mac-polish` (plus the window title after a late toolbar attach), `ndshell-dbus` (nested compositors get a private D-Bus and their process group is killed, so the user manager env and the build lock are safe), `port-ytm-fixes` (with tests), `port-gaps` (rich text, hover/animation, fill roles, canvas, SVG, focus by id, AppKit video, out-of-tree `nd new`). `scripts/linux-gate` now gives each lane its own target dir under `~/.cache/nd-build/targets/` with LRU eviction under the lock; the old shared `~/.cache/nd-build/target` (7.6 GB) is unused once running lanes have merged main.

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
  The g815 itself booted into Windows is `ssh windows` (desktop-vjmk52d): C: is nearly full, so
  build and run under `F:\nd-native` (`native-run.ps1 -Root F:\nd-native`). Leftover agent copies,
  26 `nd-*` scheduled tasks, temp dirs and this run's own files were removed there on 2026-10-14
  (19 GB); only `.nd-tools` (zig, .NET SDK) stays. A fresh native build takes about 3 minutes;
  a dev `hello.exe` needs its source tree next to it.

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
