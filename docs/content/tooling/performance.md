+++
title = "Performance"
description = "The frame budgets that nd apps hold, the numbers measured so far on e1504g, g815, and the macbook, and how to measure your own app."
+++

An nd app should use 0% CPU when idle, do work that scales with the dirty view, and present each
frame on time at the display's refresh rate: 16.7 ms at 60 Hz, 6.9 ms at 144 Hz, and 4.2 ms at
240 Hz. The slowest test machine, e1504g (Intel i3-N305, integrated GPU, 60 Hz), sets the budgets.

## Budgets

These budgets apply on e1504g to `examples/hello` and `examples/notes` with demo data:

| Measure | Budget |
| --- | --- |
| Cold start to first frame | under 300 ms |
| Idle CPU over 60 s, focused and unfocused | 0.0% |
| RSS | under 80 MB with the GL path patched out |
| Key to present, p95 | under 16.7 ms |
| Scrolling 10k rows | no missed frames |
| 100k rows | within budget |

## Measured so far

The floor is a raw GPUI window on e1504g with no nd code (2026-10-13). Its first frame takes 106 ms
median, idle CPU is 0.000%, and RSS is 177 MB, about 100 MB over budget because the GL path is
still linked. The frame interval p95 is about 17.0 ms, with 0 of about 600 frames missed. For the
numbers and the steps to rerun them, see [baselines.md](../../../docs/plan/baselines.md).

The 100k-row scroll of `examples/scroll` on e1504g at 60 Hz, build 3bebe63b (2026-10-14), has p50
16.65 to 16.67 ms and p95 17.00 to 17.05 ms. It misses 1 to 2 of about 600 frames: the first ones,
while rows and glyphs are built. Before two fixes, it missed 153 to 253 frames at p95 52 to 100 ms.
The first fix was in the flight recorder, which serialized the whole semantic tree to JSON every
frame and now keeps typed records. The second was in the GPUI presenter, which laid out overscan
rows every frame and now lays out only visible and unmeasured rows. On g815, headless and in release, the same list
costs p95 0.9 ms of work per frame at 40 points per frame and 1.7 ms at 400.

g815 drives 240 Hz and 144 Hz monitors, and the per-monitor pacing patch was checked on both
([STATUS.md](../../../docs/plan/STATUS.md)). Windows per-monitor pacing ran on
hardware on 2026-10-14 at 240 Hz and 144 Hz. The macbook has no recorded frame budget yet. Its one measured number is the dev build
loop: an incremental `hello` build takes about 1.25 s, so the dev profile stays on LLVM instead of
Cranelift.

## Scroll

`nd diag scroll` launches the `scroll-bench` example, scrolls a virtualized list of variable-height
rows every display frame, and reads back the host's frame line.

```sh
nd diag scroll --rows 100000 --seconds 10 --step 40
```

It passes when no frame is missed beyond a warm-up allowance and p95 stays within a quarter period
of the display's rate. When the binary isn't next to `nd`, name it with `--bin` or `ND_SCROLL_BIN`.
For how the list works, see [virtualization](../guides/virtualization.md).

<!-- docs:example scroll -->

## Environment knobs

The GPUI host prints `nd-budget:` lines when these are set:

- `ND_ANIMATE=<seconds>`: re-render every window's root every display frame, then print
  `frames n= refresh_ms= p50_ms= p95_ms= p99_ms= max_ms= missed=` and quit.
- `ND_EXIT_AFTER_FIRST_FRAME=1`: print the first-frame time and quit.
- `ND_BUDGET_CLICKS=<n>`: click the first button `n` times and print the input-to-frame delay.

`nd diag budget` isn't implemented. Use these variables directly, or use `nd diag scroll`.

## Per-display pacing

Each window has its own clock at the refresh rate of the display that it's on, and an idle window
asks for no frames. To see this, move the two windows of `examples/pacing` to monitors with
different rates. Each window shows its own rate. `ND_PACING=1` prints the measured rate per window.

<!-- docs:snippet examples/pacing/src/lib.rs#clock -->

## The fast feature

`fast` is a feature of the `nd` crate (and of `nd-gpui`). It builds the GPUI backend on gpui-fast
instead of gpui-pre and is off by default. When a Linux or Windows app misses frames, enable it in
the app's dependency to compare:

```toml
nd = { version = "*", features = ["fast"] }
```

`docs/plan/baselines.md` tracks the comparison against gpui-pre. It has no result yet.

## Profiling

The frame timing histograms in every incident bundle's `profile.json` ([diagnostics](diagnostics.md))
are always on. For a timeline, build `nd-diag` with the `tracy` feature and attach Tracy. The
`nd::trace!` macros that ARCHITECTURE.md 9.1 describes haven't shipped yet.

`nd::hud::FrameHud` is the in-app frame HUD, and it reads the same recorded frames. Press F12 to
cycle it through hidden, minimal (frames per second), and full (frame time against the display
interval, the slowest frame, and dropped frames). Put the view in the window's overlay and handle
the action at the root:
`scaffold().on_action(frame_hud_toggle(&hud)).overlay(hud.clone())`.

## Rules that keep an app inside the budget

The rules for agents are in [agents/performance](../agents/performance.md). In short: virtualize
long lists, keep views small so that a change repaints only its own view, and never block the main
thread.
