# Handoff, 2026-10-18

Work paused at the user's request to save Claude Code usage (weekly limit resets Tuesday
2026-10-20). Resume from here. Read first: `docs/plan/PROMPT.md` (verbatim request plus every
dated decision), `docs/plan/STATUS.md`, `docs/plan/TODO.md`, `docs/dev/setup.md` (machine rules).

## Where things are

`main` on GitHub (FormalSnake/NativeDesktop) is tagged `v0.6.1` (e319443e). Every lane from
2026-10-16 to 18 is merged: msg-gaps, msg-linux, nd-emoji, nd-video, ci-fix, ci-fling, docs-prose.
No lane branch holds unmerged nd work. The docs site runs v0.6.1
(https://nativedesktop.formalsnake.dev, deployed by hand with wrangler on 2026-10-18).

CI on main: the macOS job is green. The Linux job fails one step, `nd docs check --web`: in the
runner's headless Chrome every embed stays one flat colour, with a SwiftShader WebGPU adapter and
no console errors. The same check renders all five embeds on the macbook (65 colours each). The
check now polls for the first frame for 30 s and prints the GPU probe and page console on failure.
Next idea: run that Chrome under `xvfb-run`, or check whether the WebGPU surface presents at all
with SwiftShader. The site no longer deploys from CI: the Cloudflare Pages project `nativedesktop`
builds every push to main through its GitHub integration, running `scripts/pages-build`.

The Linux layout goldens (`gate_m6::gpui_layout_goldens`) differ 0.6% on e1504g, on main as well;
they were recorded on g815. Not a regression.

## Ports

- messages (`~/Developer/ecosystem/messages`): branch `nd-port` (worktree `messages-nd`, macOS) and
  `nd-port-linux` (worktree `messages-nd-linux`, Linux and Windows); both merged with each other
  and pushed. Their nd path deps point at `../nativedesktop-wt-msg-gaps`, which tracks main.
  Builds run on the macbook (`target/release/messages`, real config), on e1504g (demo data,
  `MESSAGES_LINUX_HOST=e1504g ND_DIR=<nd worktree> scripts/linux-gate`), and on the Windows PC
  (`C:\nd\msg`, task `ndmsgrun`). Parity is tracked in `docs/nd-parity.md` there.
  Open on macOS: a double-click on a bubble selects a word
  (Messages does nothing); the header hover fade drives the private `NSScrollPocket`.
  Open on Linux: a .MOV in the user's own session not yet seen playing (GStreamer now in the
  app's dev shell and package); the "+" menu not yet clicked by hand.
- youtubemusic (`youtubemusic-nd`, branch `nd-port`): must run on the Kopuz daemon, not `formalmusicd` (user, 2026-10-20: their changes are upstream). nativebrowser unchanged since 2026-10-16.

## Upstream GPUI PRs (zed-industries/zed, user's account)

#65728 macOS, #65729 Windows, #65730 Linux: CLA signed, descriptions final. #65730's X11 paragraph
reports the measured runs without a before/after claim (the user chose not to wait for more).
Replies on these PRs are the user's to write.

## Machines

- Nights (user sleeps next to g815): both Linux hosts in FormalShell overnight mode and `power-saver` (`sudo -n powerprofilesctl set power-saver`; plain powerprofilesctl over ssh is refused), set 2026-10-20. Work continues as normal at nice 19, two cargo jobs.
- Compile on g815 by default (user, 2026-10-20): the macbook runs other projects' iOS simulators and our builds slow them. The macbook builds only what must be AppKit or Swift, under `taskpolicy -b` with `CARGO_BUILD_JOBS=2`, one at a time.
- macbook: low priority, macOS crates only, `-p`, one heavy build at a time.
- g815: NixOS when on Linux; builds only through `scripts/linux-gate`. It was booted into Windows
  most of 2026-10-17 and is off now.
- e1504g: the user allowed cargo builds and `nixos-rebuild` on it for this session (2026-10-17).
  Its dev-shell env drvs are built locally with `--max-jobs 1 --option builders ""` because its
  remote builders point at g815. Its system runs `~/.config/nix` ae6c39b2 (Plasma logout ends the
  graphical session, Steamed theme in Plasma, panel-gamma only under Hyprland).
- Windows PC (g815 booted into Windows, `ssh windows`): Rust MSVC toolchain; agent dirs `C:\nd\msg`,
  `C:\nd\video`, `C:\nd\emoji` with their scheduled tasks are still there.
- Windows VM on g815 (the same bare-metal install as a KVM guest, so Windows work no longer
  needs a reboot): `ssh g815 'sudo systemctl start windows-vm'`, then
  `ssh -J g815 -p 2222 kyan@127.0.0.1`; VNC on g815's 127.0.0.1:5901. Its writes land in an
  overlay recreated on every start, so nothing copied in survives a stop. Monitor disk space
  throughout: the overlay grows in `/var/lib/windows-vm` on g815's root (shared with the Linux
  target dir), and the launcher refuses to start under 15 GB free. Check
  `ssh g815 'df -h / ; du -sh /var/lib/windows-vm'` and the guest's `Get-PSDrive C` before and
  after every build or copy; stop and report rather than clean when it runs low.
  About 12 minutes after a start `windows-vm-prep` reboots the guest into a local admin `ndvm`
  (pagefile off, autologon): wait for it (`systemctl status windows-vm-prep`) before building,
  or the reboot kills the build. GUI tests run as a scheduled task registered over
  `ssh -p 2222 ndvm@127.0.0.1` (a task of `kyan` does not start, `kyan` holds no session), and
  ndvm's first login shows the privacy screens over everything until they are clicked through
  (an elevated task: `mouse_event` into them is refused otherwise). The VM has no Edge, so the
  PDF association is broken there. With the pagefile off, three rustc jobs run the 8 GB guest
  out of memory (os error 1455); two fit. A debug `cargo test --workspace` grew the overlay by
  about 30 GB and took g815 to 15 GB free: build with `CARGO_PROFILE_DEV_DEBUG=0`. A guest
  `Optimize-Volume -ReTrim` does not shrink the overlay file.

## Rules that bit us (keep them)

No `rm`/`mv` on variable paths; never `pkill -f` by pattern; never git reset/checkout/restore/stash;
one heavy macbook build at a time; every merge pushed to GitHub main. Agents must never type into
the real Messages.app or the port running on the user's real account: composer and send tests run
only in a `MESSAGES_DEMO=1` instance. A dev shell's `LD_LIBRARY_PATH` must not reach system
binaries (it broke git and Chrome on CI); wasm code uses `nd_core::time`, never `std::time`.

## Scheduled resume

A launchd job (`~/Library/LaunchAgents/dev.formalsnake.nd-resume.plist`, removes itself after it
runs) fires 2026-10-20 14:00, or at the next wake after that: `scripts/resume-in-herdr` starts a fresh
`claude` session in Herdr pane `w1F:p3` (a new workspace if that pane is gone) with
`docs/plan/RESUME-PROMPT.md` as its first prompt. Log: `~/Library/Logs/nd-resume.log`.
