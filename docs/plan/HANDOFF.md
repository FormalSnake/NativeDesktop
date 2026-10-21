# Handoff, 2026-10-21

Parked at the user's request. Resume from here. Read first: `docs/plan/PROMPT.md` (verbatim request
plus every dated decision), `docs/plan/STATUS.md`, `docs/plan/TODO.md`, `docs/dev/setup.md`
(machine rules), and the platform guidelines in `docs/dev/hig/{macos,linux,windows}.md`.

## Where things are

`main` on GitHub (FormalStudio/NativeDesktop, moved from FormalSnake on 2026-10-21) is at
`ec44b91d`. Merged today: nd-tabs (tab drag across lists and windows, edge-revealed sidebar),
windows-hig (Fluent look, field headers, selection), macos-hig (Show/Hide Sidebar menu role on
control-command-S, F9 elsewhere, `nd::ToggleSidebar`; palette keeps the keyboard on refocus),
cef-extensions-2 (Web Store installs by id, button and cancel; Origin fix; `docs/plan/upstream-cef.md`),
msg-linux-2 (drawn submenus, colour emoji patch 0013, video release, scroll in popovers, pane
slides), formalmusic-ui (player bar and shelf work), gate-hardening, cef-webview-more (print,
print_to_pdf). The FormalStudio repoint of links, badges, CI and docs is done in nd and the app repos.

Known red on main: `cargo test -p nd-backend-gpui --test media` (macOS golden `media-light`,
2.88% off). The new player bar squeezes the now-playing title and artist to "Pap..." and
"The...". The FormalMusic lane owns the fix (centre block takes the free width, then re-record
the media goldens on all three OSes).

## Open lanes (each in its own worktree, branch named after the lane)

- **FormalMusic UI** (`youtubemusic-nd`, nd worktree `nativedesktop-wt-fmui`): fix the media
  golden squeeze above; match Apple Music on macOS (macOS 26 and 27 references, floating Liquid
  Glass player pill as an nd capability, Linux keeps a docked GNOME bar, Windows a WinUI bar);
  player bar: artwork and the title/artist block aligned as Music.app, and the whole centre block
  (artwork, text, seek line) vertically centred in the pill (user, 2026-10-21: it sits low).
  Report measured gaps and side-by-side crops. Then commit (fixes, then `chore: clippy`), point
  FormalMusic's nd deps at `git = "https://github.com/FormalStudio/NativeDesktop", branch = "main"`
  (local commit `build: take nativedesktop from github...` exists on nd-port, still on branch
  `formalmusic-ui`; switch to main), check a fresh clone builds (macOS and g815), pin the flake,
  push nd-port and main. Source URLs for the references go in `docs/parity.md`.
- **Messages** (`messages-nd` macOS, `messages-nd-linux` Linux; nd `nativedesktop-wt-msg-linux-2`):
  staged, uncommitted in `messages-nd`: nd deps and gpui patches on git main, `nix/package.nix`
  fetching nd from the `Cargo.lock` rev. Next: `cargo update -p nd` and the gpui crates,
  fresh-clone build on macOS and g815, flake pin, push nd-port and main. Local commit aa842f5 on
  nd-port-linux drops `MESSAGES_ICON_PATH`. The tray "?" fix (nd looks the icon name up in the
  icon themes including the app's nix share dir, else sends its own icon as pixmap; fixes every
  nix-installed nd app) is uncommitted in both worktrees and needs one gate run (tray against
  waybar, video test, cold start 597), then commit. Remaining parity: colour emoji stills,
  drag-to-attach by hand, rustls, Linux CSD and app id.
- **nativebrowser** (lynk-browser): local commit `ff4aaae` points nd and the 10 gpui patches at
  NativeDesktop main (no tag, user 2026-10-21; `Cargo.lock` pins `108d728d`, newer via
  `cargo update -p`). `dev.star` and `fspec.lock` unchanged. Fresh-clone build: macOS running in
  `~/Developer/tmp-fresh/nb-mac2`, Linux under the gate on steamlib. Push main when both pass.
- **linux-hig** (`nativedesktop-wt-linux-hig`): seven local commits (theme, variable-font weight
  fix for Adwaita Sans in the vendored cosmic text system, window controls, GPUI controls,
  ndshell, docs, goldens), rebased on 2180ccf4, needs a rebase on main and one gate (clippy,
  re-record GPUI goldens, `nextest --workspace`), then push. Nautilus captures show g815's
  "FormalDisk" volume and stay out of the commit.
- **cef-extensions-2**: Linux extension popups (uncommitted, `crates/nd-cef/src/linux/chrome.rs`
  reports the popup on placement) need one gate run, then commit
  `feat(cef): extension popups on linux in views-hosted pages`. The 20-cold-start macOS gate needs
  a rerun at low load. 1Password popup, desktop-app link and fill need the user to unlock
  1Password and approve the browser (may need a Developer ID signed `nd package`).
- **windows-hig**: Alt access keys (needs an nd-ui access-key property, key-tip overlay and
  dispatch; plan in the tracker) and the window menu when title-bar setup fails (needs a VM run
  forcing the failure). VM captures pending in `docs/parity/windows-hig.md`.

## Queued, not started

1. docs-captures lane: a Windows transport for `nd docs capture` (Unix socket only today,
   `crates/nd-cli/src`), then recapture every example on macOS, Linux and Windows into
   `docs/app/assets/captures/<example>/`.
2. nd main stabilisation: fix broken tests (the media golden first), bring the 47-minute suite
   under 10 minutes.
3. Update STATUS.md to today; close draft PR #1.
4. After the user's next Windows login: verify `messages-install.log`, install FormalMusic there.
5. Deploy every modified app to every machine: flake update, rebuild the macbook, g815, e1504g
   (from g815 with `--target-host`), and the real Windows install.

## Upstream GPUI PRs (zed-industries/zed, user's account)

#65728 macOS, #65729 Windows, #65730 Linux: CLA signed, descriptions final. #65730's X11 paragraph
reports the measured runs without a before/after claim (the user chose not to wait for more).
Replies on these PRs are the user's to write.

## Machines

- g815 build gate (2026-10-21): `scripts/linux-gate` keeps the lock, holder file and
  `nix develop --profile` GC roots on root (`~/.cache/nd-build`), and puts targets and the CEF
  cache on `/mnt/steamlib/nd-build` (`ND_LINUX_BUILD_ROOT`; NTFS via ntfs3, works for cargo).
  Caps: 80 GB of targets and a 30 GB floor on steamlib (user: use it, clean up properly), a
  5 GB floor on root, and a 30-minute hold per run (`ND_LINUX_GATE_TIMEOUT_MIN`, prints
  `ND_GATE_TIMEOUT`). Waiters print the holder. Root is shared with FormalSpec work
  (`~/fspec-*`, `~/build/formalspec-iso*`, the growing `~/fspec-bootc-vm/disk.raw`): never touch
  it; when root runs out because of it, stop and tell the user. At parking: root 22 GB free,
  steamlib 114 GB. The Windows VM starts only with 45 GB free on root.
- Nights (user sleeps next to g815): both Linux hosts in FormalShell overnight mode and `power-saver` (`sudo -n powerprofilesctl set power-saver`; plain powerprofilesctl over ssh is refused), set 2026-10-20. Work continues as normal at nice 19, two cargo jobs.
- Compile on g815 by default (user, 2026-10-20): the macbook runs other projects' iOS simulators and our builds slow them. The macbook builds only what must be AppKit or Swift, under `taskpolicy -c utility` with `CARGO_BUILD_JOBS=2`, one at a time (user, 2026-10-21: `-b` starved builds to 1 min CPU per 38 min wall).
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
