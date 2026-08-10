# libghostty-vt provenance

The two static archives under `lib/` are prebuilt, not built by this repo's
build system:

- `lib/libghostty-vt-macos-aarch64.a`
- `lib/libghostty-vt-linux-x86_64.a`

Upstream: [ghostty-org/ghostty](https://github.com/ghostty-org/ghostty), MIT
(see `LICENSE` in this directory). libghostty-vt is versioned separately from
the Ghostty app; the version string embedded in both archives is `0.1.0-dev`
(upstream `build.zig`'s `lib_version`, also queryable at runtime via
`ghostty_build_info` in `include/ghostty/vt/build_info.h`).

Build facts, recovered from `strings` on the archives:

- Compiled with Zig 0.15.2 (from a Nix devshell; `/nix/store/...-zig-0.15.2`
  paths appear in embedded debug strings). The archives are plain C static
  libraries, so they stay independent of this repo's pinned Zig 0.16.0.
- The exact upstream commit was not recorded when the archives were vendored
  (repo commits 54354f2 and 9e2108d). Treat `0.1.0-dev` at mid-2026 `main` as
  the closest pin.

To regenerate: clone ghostty-org/ghostty, build the `ghostty-vt-static`
artifact with a Zig 0.15.x toolchain for each target
(`aarch64-macos`, `x86_64-linux-gnu`), then copy the resulting archive over
`lib/` and refresh `include/ghostty/` from the same build. Record the upstream
commit here when you do.

Note for the Linux release build: the x86_64 archive links glibc symbols from
the machine that produced it. If the release container's glibc is older, the
final link of `nd-hello` fails on glibc version references; that is an archive
mismatch, not a GTK problem, and the fix is rebuilding the archive in an equal
or older glibc environment.
