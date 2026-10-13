# Baselines

Numbers later gates compare against (ARCHITECTURE.md 9.5, 10.5). Each section says what ran,
where, and how to rerun it.

## Raw gpui-pre window on e1504g (2026-10-13)

`crates/nd-gpui/examples/raw_window.rs`: one GPUI window with a column of text and a box, no nd
code. gpui-pre 0.3.9 (zed@93d7456), `--release`, built on g815 inside `nix develop`, binary
copied to e1504g with its runtime closure pushed by `nix copy --to ssh://e1504g` from g815.
e1504g: Intel Core i3-N305, eDP-1 1920x1080 at 60.008 Hz, Hyprland 0.56.2, Wayland
(`wayland-1`), Mesa ANV through `/run/opengl-driver` (the dev shell's lavapipe
`VK_ICD_FILENAMES` unset). The window opened focused on the visible workspace.

| Measure | Result | Budget (9.5) |
| --- | --- | --- |
| Launch to first frame drawn | 167 ms first run, 106 ms median of the next 9 (102 to 117) | under 300 ms |
| Idle CPU over 60 s, focused | 0.000% (0 ticks) | 0.0% |
| Idle CPU over 60 s, unfocused | 0.000% (0 ticks) | 0.0% |
| RSS idle (VmRSS = VmHWM) | 177 MB | under 80 MB with the GL path patched out |
| Frame interval, forced redraw every frame for 10 s, 3 runs | p50 16.66 ms, p95 17.02 to 17.06 ms, p99 17.30 to 17.40 ms, max 17.95 to 19.84 ms, 0 missed of about 600 | key-to-present p95 under 16.7 ms (measured with `wp_presentation` from M2) |

Notes for the M2 gate:
- RSS is over budget by about 100 MB. The GL path (about 59 MB, `docs/research/gpui.md` section
  1) is still linked; the patch that drops it (zed #63313) is the first candidate for the
  `nd-gpui` queue.
- Startup is measured from the launcher's timestamp to the `on_next_frame` callback after the
  first render, so it includes process exec and the Vulkan device setup. `since_main_ms` was 5 ms
  less on every run.
- Frame interval is frame-to-frame time between renders that call `request_animation_frame`,
  not present time; "missed" counts intervals over 1.5 refresh periods.

Rerun: build on g815 (`cargo build --release -p nd-gpui --example raw_window --features
platform`), copy `target/release/examples/raw_window` to e1504g, then run it with
`LD_LIBRARY_PATH` set to the dev shell's GPU libraries plus `/run/opengl-driver/lib` and
`XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`, `HYPRLAND_INSTANCE_SIGNATURE` from the session.
`ND_RAW_EXIT_AFTER_FIRST_FRAME=1` prints the first-frame timestamp and quits;
`ND_RAW_ANIMATE=10` prints the frame interval line; with neither it stays open idle for the CPU
and RSS samples (`/proc/<pid>/stat` fields 14 and 15, `/proc/<pid>/status`).

## Dev build: LLVM vs Cranelift on the macbook (2026-10-13)

An incremental edit to `examples/hello/src/main.rs` (one string literal changed), then
`cargo build -p hello`, everything else warm. LLVM: the pinned 1.95.0. Cranelift:
`nightly-2026-10-01` with `rustc-codegen-cranelift-preview`, `-Zcodegen-backend`,
`profile.dev.codegen-backend = "cranelift"` for workspace crates and LLVM for dependencies, in its
own build dir. Two rounds of four edits each, alternating backends. The machine was shared with
other lanes' builds (load average 48 to 107), so single runs vary by seconds.

| Backend | Edit to built binary, 8 runs | Median |
| --- | --- | --- |
| LLVM (stable 1.95.0) | 0.89 to 1.51 s | 1.25 s |
| Cranelift (nightly) | 0.85 to 5.17 s | 1.35 s |
| Cold workspace build of `hello` on the Cranelift toolchain | 3 min 25 s (deps already fetched) | |

Decision: the dev profile stays on LLVM. `hello` is small enough that the build is dominated by
linking and the Swift aggregate, so Cranelift does not speed it up here, and it would add a nightly
toolchain and abort-on-panic on macOS (10.5). Re-measure when a workspace crate edit (nd-ui,
nd-backend-appkit) dominates the loop.

Found along the way: every `hello` build, even with nothing changed, took about 5 s because
`nd::build::app()` watched `nativedesktop.toml`, which `examples/hello` does not have, and cargo
treats a missing watched file as changed, so the Swift aggregate rebuilt every time. The build
script now watches the file only when it exists; a no-op build takes about 1 s.

Rerun: `cargo build -p hello` after the edit for LLVM; for Cranelift
`cargo +nightly-2026-10-01 build -p hello -Zcodegen-backend --config
'profile.dev.codegen-backend="cranelift"' --config 'profile.dev.package."*".codegen-backend="llvm"'`
with `CARGO_BUILD_BUILD_DIR` and `CARGO_TARGET_DIR` pointed at a separate directory (two compilers in
one build dir fail with E0514).
