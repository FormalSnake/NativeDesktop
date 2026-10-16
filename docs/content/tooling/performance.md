+++
title = "Performance"
description = "The frame budgets nd apps hold, the numbers measured so far on e1504g, g815 and the macbook, and how to measure your own app."
+++

The target is idle at 0% CPU, work that scales with the dirty view, and a frame on time at every
display's refresh rate: 16.7 ms at 60 Hz, 6.9 ms at 144 Hz, 4.2 ms at 240 Hz. The slowest test
machine is e1504g (Intel i3-N305, integrated GPU, 60 Hz), so it sets the budgets.

## Budgets

Measured on e1504g for `examples/hello` and `examples/notes` with demo data:

| Measure | Budget |
| --- | --- |
| Cold start to first frame | under 300 ms |
| Idle CPU over 60 s, focused and unfocused | 0.0% |
| RSS | under 80 MB with the GL path patched out |
| Key to present, p95 | under 16.7 ms |
| Scrolling 10k rows | no missed frames |
| 100k rows | within budget |

## Measured so far

A raw GPUI window on e1504g with no nd code, as the floor (2026-10-13): first frame 106 ms median,
idle CPU 0.000%, RSS 177 MB (over budget by about 100 MB, the GL path is still linked), frame
interval p95 about 17.0 ms with 0 missed of about 600. The numbers and the rerun steps are in
[baselines.md](../../../docs/plan/baselines.md).

The 100k-row scroll of `examples/scroll` on e1504g at 60 Hz, build 3bebe63b (2026-10-14): p50
16.65 to 16.67 ms, p95 17.00 to 17.05 ms, 1 to 2 of about 600 frames missed (the first, while rows
and glyphs are built). Before two fixes it missed 153 to 253 frames at p95 52 to 100 ms. The first
fix was the flight recorder, which serialized the whole semantic tree to JSON every frame and now
keeps typed records. The second was the GPUI presenter, which laid out overscan rows every frame
and now lays out visible and unmeasured rows only. On g815, headless and in release, the same list
costs p95 0.9 ms of work per frame at 40 points per frame and 1.7 ms at 400.

g815 drives 240 Hz and 144 Hz monitors and the per-monitor pacing patch was checked on both
([STATUS.md](../../../docs/plan/STATUS.md)). Windows pacing is written and has not run on a Windows machine.
The macbook has no recorded frame budget yet; its measured number is the dev build loop: an
incremental `hello` build takes about 1.25 s, so the dev profile stays on LLVM instead of Cranelift.

## Scroll

`nd diag scroll` launches the `scroll-bench` example, scrolls a virtualized list of variable-height
rows every display frame, and reads the host's frame line back.

```sh
nd diag scroll --rows 100000 --seconds 10 --step 40
```

It passes when no frame is missed beyond a warm-up allowance and p95 stays within a quarter period
of the display's rate. `--bin` or `ND_SCROLL_BIN` names the binary when it is not next to `nd`.
The list itself is explained in [virtualization](../guides/virtualization.md).

<!-- docs:example scroll -->

## Environment knobs

The GPUI host prints `nd-budget:` lines when these are set:

- `ND_ANIMATE=<seconds>`: re-render every window's root every display frame, then print
  `frames n= refresh_ms= p50_ms= p95_ms= p99_ms= max_ms= missed=` and quit.
- `ND_EXIT_AFTER_FIRST_FRAME=1`: print the first-frame time and quit.
- `ND_BUDGET_CLICKS=<n>`: click the first button `n` times and print the input-to-frame delay.

`nd diag budget` is not implemented; use these directly or `nd diag scroll`.

## Per-display pacing

Each window has its own clock at the refresh rate of the display it is on, and an idle window asks
for no frames. Move the two windows of `examples/pacing` to monitors with different rates and each
shows its own. `ND_PACING=1` prints the measured rate per window.

<!-- docs:snippet examples/pacing/src/lib.rs#clock -->

## The fast feature

`fast` is a feature of the `nd` crate (and of `nd-gpui`). It builds the GPUI backend on gpui-fast
instead of gpui-pre and is off by default. To compare when a Linux or Windows app misses frames,
enable it in the app's dependency:

```toml
nd = { version = "*", features = ["fast"] }
```

The comparison against gpui-pre is tracked in `docs/plan/baselines.md`; no result is recorded there
yet.

## Profiling

The frame timing histograms in every incident bundle's `profile.json` ([diagnostics](diagnostics.md))
are always on. For a timeline, build `nd-diag` with the `tracy` feature and attach Tracy. The
`nd::trace!` macros described in ARCHITECTURE.md 9.1 are not shipped yet.

`nd::hud::FrameHud` is the in-app frame HUD: F12 cycles it hidden, minimal (frames per second) and
full (frame time against the display interval, the slowest frame, dropped frames), read from the
same recorded frames. Put the view in the window's overlay and answer the action at the root:
`scaffold().on_action(frame_hud_toggle(&hud)).overlay(hud.clone())`.

## Rules that keep an app inside the budget

[agents/performance](../agents/performance.md) holds the rules for agents. The short version:
virtualize long lists, keep views small so a change repaints only its own, and never block the
main thread.
