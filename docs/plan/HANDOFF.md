# Handoff, 2026-10-22

Resume from here. Read first: `docs/plan/PROMPT.md` (verbatim request plus every dated decision),
`docs/plan/STATUS.md`, `docs/plan/TODO.md`, `docs/dev/setup.md` (machine rules), and the platform
guidelines in `docs/dev/hig/{macos,linux,windows}.md`.

## Where things are

`main` is at `48b07503` (fm-hero-linux: the player cover travels above the bar both ways on
Linux; docs Worker, syntax highlighting). Earlier: `30e8e952`. Merged late on 2026-10-22: fmui-pill-nobar (`da716faf`, the macOS pill is
plain Liquid Glass with no scroll edge band behind it; the top toolbar is stock NSToolbar) and
fix-share-crash (`30e8e952`, nd's panic hook ends with `_exit(101)` so a panic off the main thread
no longer runs the GPU driver's exit handlers mid-frame). Earlier that day: fmui-pill-progress,
fm-motion, fmui-scroll, gate-toolchain, fm-blur-edges, gnome-split, cef-ext-popup-view.

Deployed through `~/.config/nix` `4b9c1032` on g815 and the macbook: FormalMusic `9a9d93a` (nd
`30e8e952`, share crash fixed, kopuzd started with `--app-id`/`--app-name`). The user's running
instances keep the old build until restarted.

Docs site: native.formalsnake.dev is served by the `nativedesktop-docs` Worker (static assets,
`wrangler.jsonc`), built by Workers Builds from FormalStudio/NativeDesktop main with
`scripts/docs-site-build` and `npx wrangler deploy`; every push to main redeploys it. The Cloudflare
GitHub app on FormalStudio was reinstalled from the CanaryCoders account on 2026-10-22 (other
Cloudflare projects building from FormalStudio repos may need reconnecting). The old Pages project
`nativedesktop` no longer has the domain and can be deleted.

The previous orchestrator session stopped on a rate limit (2026-10-22 15:00); its lanes were
restarted from their worktrees the same evening.

## Night of 2026-10-22 to 23

- Deployed: FormalMusic `cdfcd6e` (title fix 41eed81 plus hero fix, nd 48b07503) via nix `be845090` on
  g815 and the macbook. The deploy agent restarted Kyan's g815 FormalMusic (old pid 3224039) against
  the rule; kopuzd 511770 was untouched, the new instance is pid 4034736.
- Merged on nd main: fm-lyrics-glide (`cad26429`), docs-perf and docs-one-site (`2f35ddeb`: WebP shots,
  prerender on hover, live examples on their pages, search dialog, `/app/` redirects to `/`).
- fspec: nd generator and `fspec.toml` layer merged (`e00211dd`, fixture fix `de4968e1`). FormalMusic
  main and nd-port `ea01945e` and Messages main, nd-port, nd-port-linux `20f4bbf4` ship a generated
  `fspec.star` (load with `gh:FormalStudio/<app>@main//fspec.star`). nixmgr switches e1504g once
  formalspec confirms e1504g runs fspec 08549fd or later.
- now-playing merged in nd (`0a05d464`); FormalMusic release `5506019` (now-playing, lyrics) installed via nix
  `604282ab` on g815 and the macbook and relaunched on both. On g815 `nixos-rebuild switch` crashed twice
  (SIGSEGV in nix auto-GC); the built closure was activated by hand, worth a look.
- docs-captures-2 `1d6003ce` (pushed, held): Linux window controls follow button-layout in the split
  headers, Notes editor fills its column, native list selection on all three (test
  `selected_row_follows_the_platform`), Linux browser shot via headless Ozone, 2x Linux shots. Left:
  native-widget on macOS and Linux, the Linux browser shot, every Windows shot. Needs g815 online and the
  macbook unlocked. Notes' AppKit audit fails on the sidebar badge contrast, also on main.
- Merged on nd main: fm-bar-narrow (`4d84f31a`), native-widget-map (`066c0265`: drawn OSM map on
  Linux and Windows, places Antwerpen, Arrecife, Las Palmas). FormalMusic `c1bc6a8` (narrow bar, fspec)
  installed via nix `aea3aca3` on both; relaunched on the macbook. g815 dropped off the network right
  after its switch (ssh and Tailscale time out, about 06:30), so its relaunch and every g815 build or VM
  run are blocked until it is back: needs Kyan to wake or reconnect it.

## Stopped on the usage limit (2026-10-22 night)

Lanes in flight, each in its own worktree with work saved there:
- FormalMusic deploy: `formalmusic-wt-deploy2` branch deploy-hero (hero-linux merged onto 41eed81, locks
  pinned to nd 48b07503, uncommitted); next: test, nix build, push main and nd-port, nix bump, switches.
- Motion: `fm-lyrics-glide` 184e9a83 local (stacked on 48b07503), `fm-bar-narrow` rebased; Windows check
  of the hero fix not done.
- Now playing: nd fmui-now-playing 052ae26b and app now-playing 2d0dcbf rebased locally, not pushed;
  scrim 60 to 70 percent at app `crates/app/src/root.rs:64` uncommitted, g815 look run queued.
- docs-captures-2 (list selection, Notes editor, full recapture), toolbar-search (centred search,
  header padding and centred title on Linux), native-widget-map (Antwerpen, Arrecife, Las Palmas;
  drawn map), docs-perf then docs-one-site, build-speed (nd and apps), fspec (nd .star generator;
  formalspec 08549fd has gh: loads and origin(); e1504g needs that fspec first), Kopuz PR
  `mpris-app-identity` merged as Kopuz-org/kopuz#807 (`d0c915a9`): kopuzd applies `--app-id`/`--app-name`
  to MPRIS on Linux; Kopuz itself still reports "Kopuz". Not yet checked on a real Linux desktop.
Needs Kyan: delete the old Pages project `nativedesktop` (domain already moved), a Kopuz release
with #807 so the flake can move past v0.22.0.

## Open lanes (each in its own worktree, branch named after the lane)

- **FormalMusic motion** (`nativedesktop-wt-fm-motion` branch fm-hero-linux, `formalmusic-wt-motion`
  branch hero-linux): on Linux the expanded player opens without animation and the cover slides
  behind the bar on close (user, 2026-10-22, repeated that evening). Fix: start the open the same
  way as macOS and draw the moving cover above the bar both ways. Then fm-lyrics-motion (smooth
  glides, unblur on hover or scroll, match FormalShell and Kopuz), then fm-bar-narrow (bar controls
  overflow at 480 wide) plus clippy too_many_arguments at nd-backend-gpui nav.rs:351 and :1546.
- **FormalMusic now playing** (`nativedesktop-wt-fmui` branch fmui-now-playing a3fd5e07,
  `youtubemusic-nd` branch now-playing dfa4496): playing row with equalizer bars, "Playing from"
  header, darkened player backdrop. macOS done; g815 Adwaita and the Windows 11 VM next, then merge.
- **FormalMusic title** (`nativedesktop-wt-fm-title` fm-title, `formalmusic-wt-title` fix-title):
  on Linux the content header keeps "FormalMusic" as its title on every page (user, 2026-10-22).
  The app sets it statically at `crates/app/src/root.rs:821`.
- **docs-captures-2** (6934bbf4, held, agent stopped on the rate limit): Linux window controls must
  follow the system button layout (GNOME default is close only) and sit in the gnome-split headers;
  check whether `real_adblock_cef` fails on main too; recapture browser on macOS; make the Windows VM
  zig cache copy a setup step. Not restarted yet.
- **Kopuz MPRIS identity**: Kopuz-org/kopuz branch `mpris-app-identity` (`67d504c8`) makes kopuzd
  apply `--app-id`/`--app-name` to MPRIS on Linux. Shared repo: the user decides on a PR and release;
  the flake moves past kopuz v0.22.0 after that.
- **Messages**: idle; bump to each new nd main when asked. Remaining parity: colour emoji stills,
  drag-to-attach by hand, rustls, Linux CSD and app id.
- **windows-hig**: a VM check that a lone Alt reaches GPUI, and the window menu when title-bar
  setup fails.
- **FormalMusic automation tree**: on g815 `getTree` lists 2 nodes before the library loads, and the
  installed build lacks nd-backend-gpui's `capture` feature, so deploy checks cannot screenshot.

Every FormalMusic branch is checked on g815, macOS and the Windows 11 VM before merging (user,
2026-10-22). Never grim or click the user's real outputs: visual checks use a separate headless
output or the app's own screenshot test.

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

Relaunch after install (Kyan, 2026-10-23): after installing a new release of an nd app, quit the running
instance by PID and relaunch the new build on that machine. kopuzd keeps running.
Macbook disk (2026-10-22): app worktrees (FormalMusic, Messages) build into their own `target/` on
the internal disk, and nine of them filled it (ENOSPC, three lanes stopped). Heavy builds go to
g815; a lane runs `cargo clean` in its own worktree when it finishes.
No `rm`/`mv` on variable paths; never `pkill -f` by pattern; never git reset/checkout/restore/stash;
one heavy macbook build at a time; every merge pushed to GitHub main. Agents must never type into
the real Messages.app or the port running on the user's real account: composer and send tests run
only in a `MESSAGES_DEMO=1` instance. A dev shell's `LD_LIBRARY_PATH` must not reach system
binaries (it broke git and Chrome on CI); wasm code uses `nd_core::time`, never `std::time`.
