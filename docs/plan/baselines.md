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

## 100k-row scroll, M5 (2026-10-13)

`nd diag scroll --rows N --seconds 10 --step S` launches `scroll-bench` (`examples/scroll`): a
virtualized list of rows 28 to 124 points tall, scrolled `S` points every display frame through
`ListState::scroll_by`, under the GPUI host's `ND_ANIMATE` frame line. It passes at no more than 2
missed frames and p95 within 1.25 periods.

| Where | Build | Rows, step | Result |
| --- | --- | --- | --- |
| e1504g, eDP-1 60 Hz, window | 0c485261 (before the fixes below) | 100k, 40 | p95 52.7 to 100.6 ms, missed 153 to 253 of 154 to 269: over |
| e1504g | 0c485261 | 10k, 40 | p95 100.6 ms: over, so per-frame cost, not row count |
| g815, headless, release | 8d03a1c5, again on ee5049c6 (router scrolling) | 100k, 40 | p95 0.87 to 0.94 ms, max 1.43 per frame, 0 over 16.7 ms |
| g815, headless, release | same | 10k, 40 | p95 0.90 ms |
| g815, headless, release | same | 100k, 400 | p95 1.69 ms, max 2.06 |
| e1504g, eDP-1 60.008 Hz, window, Hyprland session | 3bebe63b (main, both fixes) | 100k, 40, 3 runs | p50 16.65 to 16.67 ms, p95 17.00 to 17.05, p99 17.46 to 17.74, missed 1 to 2 of 598 to 601: within |
| e1504g | 3bebe63b | 10k, 40, 2 runs | p50 16.66 to 16.67 ms, p95 17.02, missed 1: within |
| e1504g | 3bebe63b | 100k, 400 | p50 16.67 ms, p95 17.02, missed 1: within |

The two fixes: `nd-diag`'s recorder kept each frame's semantic tree as a `serde_json::Value`
(about a quarter of main-thread samples on the macbook, far more on e1504g); it now keeps typed
records and serializes when a bundle is written. The GPUI presenter laid out every rendered row
each frame, overscan included; it now lays out visible rows and rows not yet measured at the
current width. With both, e1504g holds 60 Hz at 100k rows (2026-10-14); the missed frame is the first
one, while rows and glyphs are first built.

Rerun: build on g815 (`cargo build --release -p scroll-bench -p nd-cli`), stage both binaries and
the dev shell's `LD_LIBRARY_PATH` on e1504g, then `nd diag scroll --rows 100000 --bin ./scroll-bench`
with the Hyprland session's `WAYLAND_DISPLAY`. Headless work budget anywhere:
`cargo nextest run --cargo-profile release -p nd-backend-gpui --test scroll_budget` (`ND_ROWS`,
`ND_SCROLL_STEP`, `ND_FRAMES`).
