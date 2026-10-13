# Dev setup

## macbook

Needs Xcode or the Command Line Tools (Swift and the macOS SDK come from the system, never from Nix), Nix with flakes, and the cache drive mounted at `/Volumes/Kyan Drive`.

```sh
git clone <repo> nativedesktop && cd nativedesktop
git add -N .                      # flakes only see tracked files; repeat after adding files
nix develop                       # or: direnv allow, if .envrc exists
rustup show                       # installs the toolchain pinned in rust-toolchain.toml
cargo nextest run --workspace --exclude nd-backend-appkit   # tiers 1 and 2
cargo test -p nd-backend-appkit   # harness = false binaries, nextest cannot list them
cargo test --workspace            # also runs the harness = false AppKit binaries
```

A cold build takes about 5 minutes (longer while other builds run). Intermediate artifacts go to `/Volumes/Kyan Drive/dev-cache.noindex/cargo-target/nativedesktop/` through `build.build-dir`; `scripts/rustc-guard` fails the build when the drive is missing. Without the drive: `CARGO_BUILD_BUILD_DIR=$PWD/target/build ND_ALLOW_LOCAL_BUILD=1 cargo ...`.

The shell puts rustup's proxies first, exports no `SDKROOT` or `DEVELOPER_DIR`, and puts `/usr/bin` ahead of every nix cc, cctools, binutils and `xcrun` (also ones inherited from the calling environment). Check with `command -v rustc xcrun strip` (expect `~/.cargo/bin/rustc`, `/usr/bin/xcrun`, `/usr/bin/strip`). The strip matters: rustc runs `strip` on release builds, and a nix strip corrupts proc-macro dylibs ("mis-aligned LINKEDIT"), so plain `cargo build --release` works inside the shell and needs no `CARGO_PROFILE_RELEASE_STRIP=none`. Swift build scripts call `/usr/bin/xcrun` and `/usr/bin/swift` by absolute path, which the shell leaves alone. If a build fails with E0514, two compilers touched the same build dir: leave the shell, then `cargo clean`.

## Linux

Nix with flakes; a Wayland or X11 session is only needed for real-window tests.

```sh
git clone <repo> nativedesktop && cd nativedesktop
git add -N .
nix develop
rustup show
cargo nextest run --workspace --exclude nd-backend-appkit
```

The shell carries the Wayland, X11, xkbcommon, Vulkan, fontconfig and ALSA libraries GPUI loads, mold as linker, and mesa's lavapipe as `VK_ICD_FILENAMES` for headless rendering. Do not build on e1504g; build on g815 and push the closure.

## Test profiles

`.config/nextest.toml`:

| Profile | Runs |
| --- | --- |
| `default` | tiers 1 and 2, everything except tests named `real_*` or in a `real` test binary |
| `fast` | same, stops at the first failure |
| `ci` | same, no retries, JUnit output |
| `real` | tier 3 only, one at a time (`real-window` group) |
| `gate` | everything, the `real-window` group still serialized; used as `cargo nextest run --profile gate -E 'test(/^gate_mN::/)'` |

## CI

`.github/workflows/ci.yml` runs on macOS and Linux: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run --profile ci`, doc tests. Runners override the build dir with `CARGO_BUILD_BUILD_DIR` and `ND_ALLOW_LOCAL_BUILD=1`.
