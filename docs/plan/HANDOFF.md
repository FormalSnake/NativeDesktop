# Handoff, 2026-10-22

Resume from here. Read first: `docs/plan/PROMPT.md` (verbatim request plus every dated decision),
`docs/plan/STATUS.md`, `docs/plan/TODO.md`, `docs/dev/setup.md` (machine rules), and the platform
guidelines in `docs/dev/hig/{macos,linux,windows}.md`.

## Where things are

`main` is at `acc49740`. Later on 2026-10-22: fmui-pill-progress, fm-motion (`player_layout`,
heroes, `image().blur()`, lyrics depth of field), fmui-scroll (scroll edges, full-bleed shelves;
media goldens re-recorded), gate-toolchain, fm-blur-edges (blur decodes at 8 px per radius, edges
replicated), gnome-split (one header bar per pane), cef-ext-popup-view (Linux extension popups in
an nd view). g815 runs FormalMusic 9fa5817 (nix b80fbe1f; Replay Mix plays the clicked row).
Open: share crash plus playback with no UI (new lane), fmui-pill-nobar (no blur band behind the
macOS pill, user 2026-10-22), fmui-now-playing (playing row, Playing from header, Up next
contrast), motion queue fm-hero-linux, fm-lyrics-motion (match FormalShell and Kopuz), fm-bar-narrow
(plus nav.rs clippy), docs-captures-2 held (Linux controls must follow button-layout), Windows 11 VM
check for every FormalMusic branch (user 2026-10-22). Never grim the user's real outputs.

Earlier, at `8249438c`. Merged before that:
fmui-pill-clear (macOS player pill is plain system Liquid Glass again, no tint), msg-linux-4
(drawn submenus open beside their parent, flip, clamp), docs-captures (all 13 examples captured on
macOS, Linux and Windows, light and dark; Windows transport for `nd docs capture`; Windows frame
follows the app theme), gnome-polish (gpui patch 0014: hover on elements without an id, which fixed
the lagging track-row hover; libadwaita backdrop and WinUI inactive title row; 47 px header, Nautilus
sidebar colour; `ndshell shot` seat has a keyboard). The media golden is green again.

Deployed on g815: Messages `c965067d` (nd 786b1692, submenu fix confirmed in a demo instance),
FormalMusic `3a89e939` (sidebar playlist covers, confirmed by the user).

## Open lanes (each in its own worktree, branch named after the lane)

- **FormalMusic UI** (`youtubemusic-nd` nd-port, nd branches `fmui-*`): pin FormalMusic to nd
  `8249438c`, nix build, push, then deploy to g815. `fmui-pill-progress`: thin grey seek line on the
  pill (2 pt, 4 pt on hover, no accent) as Music.app, capture light and dark. Then scrolling, framework
  level: content scrolls under the toolbar and the pill with the macOS scroll edge effect (likely by
  hosting the pill as a bottom split-view accessory), last row clears the pill, GNOME header shade and
  undershoot, WinUI equivalent; full-bleed horizontal shelves and chip rows (scroll to the edge,
  first card aligned with the page title at rest). Check every main page, before/after captures.
- **FormalMusic motion** (`formalmusic-wt-motion` branch motion, `nativedesktop-wt-fm-motion` branch
  fm-motion, uncommitted): hover affordances, lyrics depth of field, expanded-player hero transition
  (overlapped open, reverse close), artwork-blur backdrop (`image().blur()`), `player_layout` for
  narrow windows (art header under 760 pt), `ND_ANIMATION_SLOWDOWN`, no black frames in animated art
  (image keeps the old picture until the new decode lands), macOS shelf hover tracking fix. macOS done;
  Linux run on g815 next (widths down to the minimum, 240 Hz black-frame check), then rebase on
  main, push both, merge, deploy.
- **Messages**: bump to nd `8249438c`, push main, nd-port, nd-port-linux, deploy to g815. Remaining
  parity: colour emoji stills, drag-to-attach by hand, rustls, Linux CSD and app id.
- **cef-extensions-2**: Linux extension popups redesigned (user, 2026-10-22: "Popup in nd view"): the
  action popup page opens in an nd webview popover anchored to the app's button, sized from the page,
  `chrome.tabs.query` sees the app's active tab. Test 1Password, uBlock Origin, Dark Reader on g815.
  The X-window approach (local wip `37b4d281`) is dropped. macOS 20-cold-start gate passed. The CEF
  "Timeout of new browser info response" is Chromium's own, once per launch, harmless; noted in
  `docs/plan/upstream-cef.md`.
- **docs-captures**: notes editor column is a narrow card on Linux and Windows; Linux shots lack
  CSD window controls; Windows browser/terminal shots show placeholders (omit browser on Windows
  until WebView2, open a terminal tab). Recapture, push, merge.
- **windows-hig**: access keys merged; a VM check that a lone Alt reaches GPUI, and the window menu
  when title-bar setup fails.

## Queued, not started

1. nd main stabilisation: bring the 47-minute suite under 10 minutes.
2. Update STATUS.md to today; close draft PR #1.
3. After the user's next Windows login: verify `messages-install.log`, install FormalMusic there.
4. Deploy every modified app to every machine: macbook, g815, e1504g (from g815 with
   `--target-host`), and the real Windows install.
5. The nd repo's committed `.cargo/config.toml` still sets `build-dir` on `/Volumes/Kyan Drive`
   (removed from nativebrowser in 88f258c); offered to move it to the ecosystem-level config, no
   answer yet.

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
  it; when root runs out because of it, stop and tell the user. Last check: root 31 GB free. The Windows VM starts only with 45 GB free on root.
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
  overlay recreated on every start, so nothing copied in survives a stop. The overlay lives on
  `/mnt/steamlib/windows-vm` (nix config `ac1b1022`), is deleted on stop, needs 45 GB free there to
  start and powers off below 30 GB. Guest TRIM does not shrink it (ntfs3). Check
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
