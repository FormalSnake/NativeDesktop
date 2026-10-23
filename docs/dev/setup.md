# Dev setup

## macbook

Needs Xcode or the Command Line Tools (Swift and the macOS SDK come from the system, never from Nix), Nix with flakes, and the cache drive mounted at `/Volumes/Kyan Drive`.

```sh
git clone <repo> nativedesktop && cd nativedesktop
git add -N .                      # flakes only see tracked files; repeat after adding files
nix develop                       # or: direnv allow, if .envrc exists
rustup show                       # installs the toolchain pinned in rust-toolchain.toml
cargo nextest run --workspace     # tiers 1 and 2
cargo build -p hello && cargo nextest run --profile real -p nd-backend-appkit   # tier 3, moves the cursor
just gate m0                      # a milestone's gate: builds the examples, runs every gate_m0:: test
```

Tier 3 posts real HID events through `crates/ndshell`. Its preflight fails the test, naming the culprit, when a window manager that intercepts synthetic input runs (OmniWM, yabai, AeroSpace, Amethyst) or the process may not post events, and releases modifier keys a crashed run left held. TCC checks the responsible process, the terminal or `/usr/libexec/sshd-keygen-wrapper` over ssh: with SIP off, `cargo run -p ndshell -- grant <that binary>` writes its Accessibility, PostEvent and Screen Recording rows.

The AppKit test binaries own the main thread (`harness = false`) and answer nextest's `--list` and `--exact` themselves (`crates/nd-backend-appkit/tests/harness`), one test per binary.

A cold build takes about 5 minutes (longer while other builds run). Intermediate artifacts go to `/Volumes/Kyan Drive/dev-cache.noindex/cargo-target/nativedesktop/` through `build.build-dir`; `scripts/rustc-guard` fails the build when the drive is missing. Without the drive: `CARGO_BUILD_BUILD_DIR=$PWD/target/build ND_ALLOW_LOCAL_BUILD=1 cargo ...`. The shell compiles through sccache (`ND_RUSTC_WRAPPER=sccache`, cache in `dev-cache.noindex/sccache`, 30 GB). sccache keys Rust crates by their build dir, so a build dir rebuilt after `cargo clean` or an E0514 takes its crates from the cache (hello: 263 s cold, 123 s from the cache at two jobs), while a new worktree still compiles them once. `ND_RUSTC_WRAPPER=` turns it off.

The shell puts rustup's proxies first, exports no `SDKROOT` or `DEVELOPER_DIR`, and puts `/usr/bin` ahead of every nix cc, cctools, binutils and `xcrun` (also ones inherited from the calling environment). Check with `command -v rustc xcrun strip` (expect `~/.cargo/bin/rustc`, `/usr/bin/xcrun`, `/usr/bin/strip`). The strip matters: rustc runs `strip` on release builds, and a nix strip corrupts proc-macro dylibs ("mis-aligned LINKEDIT"), so plain `cargo build --release` works inside the shell and needs no `CARGO_PROFILE_RELEASE_STRIP=none`. Swift build scripts call `/usr/bin/xcrun` and `/usr/bin/swift` by absolute path, which the shell leaves alone. If a build fails with E0514, two compilers touched the same build dir: leave the shell, then `cargo clean`.

## Linux

Nix with flakes; a Wayland or X11 session is only needed for real-window tests.

```sh
git clone <repo> nativedesktop && cd nativedesktop
git add -N .
nix develop
rustup show
cargo nextest run --workspace
```

The shell carries the Wayland, X11, xkbcommon, Vulkan, fontconfig and ALSA libraries GPUI loads, mold as linker, and mesa's lavapipe as `VK_ICD_FILENAMES` for headless rendering. Do not build on e1504g; build on g815 and push the closure.

## Test profiles

`.config/nextest.toml`:

| Profile | Runs |
| --- | --- |
| `default` | tiers 1 and 2, everything except tests whose last path segment starts with `real_` or that sit in a `real` module |
| `fast` | same, stops at the first failure |
| `ci` | same, no retries, JUnit output |
| `real` | tier 3 only, one at a time (`real-window` group) |
| `gate` | everything, the `real-window` group (plus the dispatch spike, which needs its window key) still serialized; `just gate mN` runs it as `cargo nextest run --profile gate -E 'test(/^gate_mN::/)'` |

Every test that puts a window on screen or needs key status holds the machine-wide ndshell lock (`ndshell::lock::window_test`) for its duration, so several runs on one machine (other worktrees, other agents) queue instead of stealing focus. The AppKit test harness (`tests/harness`) takes it for its binaries; custom `main`s, `nd::audit` (AppKit variant) and `nd_test::launch_app` do it themselves. Headless GPUI and tier 1 tests never take it. A wait longer than 600 s (`ND_WINDOW_LOCK_TIMEOUT_SECS`) fails with the holder's pid, worktree and test name. The `real-window` nextest group still orders tests within one run. A new window test calls `ndshell::lock::window_test("<test name>")` first and does not take the lock twice.

## CI

`.github/workflows/ci.yml` runs on macOS and Linux: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run --profile ci`, doc tests. Runners override the build dir with `CARGO_BUILD_BUILD_DIR` and `ND_ALLOW_LOCAL_BUILD=1`.

## Remote Linux hosts (g815, e1504g)

- Never build on e1504g. Build on g815 and copy the binary over; delete it after measuring.
- On g815, `scripts/linux-gate` gives each lane its own target under `/mnt/steamlib/nd-build/targets` and one sccache (`/mnt/steamlib/nd-build/sccache`, 20 GB), so a lane evicted by the LRU cap and rebuilt at the same path takes its third-party crates from the cache. Check `df -h /` before large builds and stop under 15 GB free.
- Start-up, idle and frame budgets of a GPUI app without touching the desktop: `ndshell headless -- scripts/budget/headless.sh <binary>` in the dev shell (a private headless Hyprland).
- Unattended runs: the hosts power off unless g815 is in FormalShell overnight mode
  (`formalshell ipc --any-display call overnight enable`, with `XDG_RUNTIME_DIR=/run/user/1000`).
  Leave power profile, brightness and overnight state alone while it is on, and keep the fans quiet:
  `nice -n 19 ionice -c3`, `CARGO_BUILD_JOBS=2`, nextest `--test-threads 2`, one heavy job at a time.
  For a silent night also close the session apps, turn Hyprland animations, blur and shadows off
  (`hyprctl eval` with `hl.config`) and power the displays off with
  `hyprctl dispatch 'hl.dsp.dpms({ action = "disable" })'`; render only in headless Hyprland.

## Shell commands in unattended runs

Never `rm`, `mv` or `find -delete` a path built from a shell variable (`rm -f $OUT/*`): the
permission guard stops for a manual approval on each one. Use a literal absolute path, let the
program clean its own temp files (`tempfile` in tests), or at minimum write `"${OUT:?}"/*`.

Never `pkill -f` or `killall` by pattern: several agents run the same commands in sibling
worktrees, and a pattern kill ends their runs too. Kill only PIDs you started yourself.

## Keeping the macbook responsive

The macbook hosts services (BlueBubbles among them), so builds there yield: `scripts/rustc-guard`
runs every rustc at utility QoS and nice 15 (`ND_FULL_PRIORITY=1` opts out), and the dev shell caps
`CARGO_BUILD_JOBS` and `NEXTEST_TEST_THREADS` at 4. Only macOS work runs on the macbook: AppKit
crates, Swift, mac gates and goldens, `-p` on the packages you touched. Workspace-wide suites,
clippy, GPUI-backend and wasm work run on g815 through `scripts/linux-gate`.
