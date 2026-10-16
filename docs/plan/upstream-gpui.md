# Upstreaming the gpui patches

nd carries nine patches on zed (`crates/nd-gpui/patches`, applied to zed@93d7456 by
`scripts/gpui-fork`). This is the plan to land them in zed-industries/zed so the fork can go.

Each patch is rebased onto zed main (2d4c483, 2026-10-16) as its own branch on
[FormalSnake/zed](https://github.com/FormalSnake/zed). The zed worktree is
`/Volumes/Kyan Drive/dev-cache.noindex/zed-upstream`.

## Zed's rules that apply

From `CONTRIBUTING.md` and `.github/pull_request_template.md` on main:

- Sign the CLA at https://zed.dev/cla before anything can merge.
- At most three PRs open at a time. Land one before stacking more.
- Features need a GitHub discussion (or an issue a staff member confirmed) before a PR. Bug
  fixes go straight to a PR.
- Non-trivial changes need tests. Visual changes need before and after screenshots, and
  non-visual ones need benchmarks or other artifacts.
- Build and run Zed and test the change by hand before opening the PR.
- AI policy: LLM help is fine with a human who understands the work; PRs from autonomous agents
  get closed, and PR descriptions and replies to maintainers should come from the author.
- Template: Summary, Testing, Self-review checklist, then a `Release Notes:` list (`- N/A` or
  `- Fixed ...`). Titles look like `gpui_linux: Fix ...`.

Every body below ends with the line `AI assistance was used`.

## Status

| Patch | Branch | Route | State |
| --- | --- | --- | --- |
| 0001 X11 per-CRTC refresh | `gpui-x11-crtc-refresh` | PR | needs an X11 run, then a free PR slot |
| 0002 GL only when Vulkan fails | none | already upstream as zed #63313 (open, by jlucaso1) | drop when #63313 merges |
| 0003 Wayland resize configure ack | `gpui-wayland-configure-ack` | PR | open: #65730 |
| 0004 `Window::last_presentation` | `gpui-wayland-presentation-time` | discussion first (new public API) | discussion #65731 |
| 0005 Windows draw in `WM_SIZING` | `gpui-windows-wm-sizing` | PR | next when a PR slot frees |
| 0006 Windows per-monitor vblank | `gpui-windows-monitor-vblank` | PR | open: #65729 |
| 0007 Windows grayscale text | `gpui-windows-grayscale-text` | PR with screenshots | needs a Windows run and captures |
| 0008 Apple font size nudge | `gpui-apple-ligature-nudge` | PR | open: #65728 |
| 0009 Live resize swapchain | `gpui-wgpu-live-resize-swapchain` | discussion first | not upstreamable as is |

Three PRs are open, Zed's limit. Open the next (0005, then 0001, then 0007) when one merges or
closes. The CLA has to be signed at https://zed.dev/cla, then comment `@cla-bot check` on #65728.

---

## 0001: gpui_linux: Pace X11 windows at the refresh rate of their own monitor

Branch `gpui-x11-crtc-refresh`. PR: not opened yet. It needs a run on X11 with two monitors at
different rates (nd never measured X11), and a free slot under Zed's three-PR limit. On g815:
check clean, gpui_linux 51 tests pass including the new one.

Issue search: nothing filed for X11 multi-monitor refresh. Zed's own comment in
`X11ClientState` says the rate should be re-queried when the window changes screens.

```
## Summary

On X11, GPUI paces every window with the refresh rate of the first CRTC that randr lists, so on a
144 Hz + 240 Hz pair a window on the 240 Hz monitor still draws at 144 Hz (or the other way
round, depending on the order). The existing comment in `X11ClientState` already says this
should be re-queried when the window changes screens.

This picks the CRTC under the window's centre when the refresh loop starts, and restarts the
loop at the new rate on `ConfigureNotify` when the window moves to a monitor with a different
refresh. With no CRTC under the centre (off screen, or the geometry query fails) it falls back
to the first CRTC as before.

## Testing

`cargo test -p gpui_linux` includes a new unit test for picking the CRTC (edges, a disabled
CRTC, no CRTC). I ran a GPUI app on X11 with two monitors at different rates and moved a window
between them: <fill in the measured rates>.

## Self-review

- [x] I've reviewed my diff for quality, security, reliability, and performance.
- [ ] UI changes follow the [checklist](https://zed.dev/docs/development/ui-checklist). (No UI change.)
- [x] Tests cover the new or changed behavior.

AI assistance was used

Release Notes:

- Fixed windows on X11 redrawing at the first monitor's refresh rate instead of the rate of the monitor they are on.
```

## 0002: gpui_wgpu: Skip the GL backend when Vulkan works

No branch. nd's patch is the change in zed #63313 (open since 2026-10-04, not reviewed yet,
two outside confirmations). nd measured the GL stack at about 59 MB resident on e1504g (raw
gpui-pre window, 177 MB RSS before). Nothing to open; drop 0002 when #63313 merges, or add
nd's number as a comment there if it stalls.

## 0003: gpui_linux: Ack a throttled resize configure with the frame that applies it

Branch `gpui-wayland-configure-ack`. PR: https://github.com/zed-industries/zed/pull/65730
(opened 2026-10-16). On g815: `cargo check -p gpui -p gpui_linux -p gpui_wgpu` clean, gpui_linux
50 tests pass. `gpui_wgpu`'s three `wgpu_atlas` tests fail there on every branch, this one
included, without touching gpui_wgpu, so they fail on main in that environment too.

Issue search: no open issue matches. #32905 (flicker while resizing on Linux) is closed.

Body as posted:

```
## Summary

During an interactive resize on Wayland, GPUI applies at most one configure per frame. A configure that came in while throttled was acked straight away, before any buffer of the new size existed. Hyprland then drew the new window geometry (border and shadow) around the old buffer for a frame, so while dragging an edge the border ran ahead of the content.

Now a throttled configure is held and acked when the next frame applies it, so the ack always goes out with a buffer of that size. Only the latest one is kept, so a burst of configures within one frame still costs a single resize.

## Testing

Hyprland 0.56: I dragged the right edge of a GPUI window while capturing every presented frame with `ext-image-copy-capture` on a headless Hyprland. Before, frames showed the new border around the old content. After, content and border matched in every captured frame. I ran that capture on an earlier main (93d7456); this branch is the same change rebased without conflicts, and `cargo test -p gpui_linux` passes on it. There's no harness for Wayland event ordering in `gpui_linux`, so the capture is the test.

## Self-review

- [x] I've reviewed my diff for quality, security, reliability, and performance.
- [ ] UI changes follow the [checklist](https://zed.dev/docs/development/ui-checklist). (No UI change.)
- [ ] Tests cover the new or changed behavior. (See above.)

AI assistance was used

Release Notes:

- Fixed the window border running a frame ahead of the content while resizing on Wayland.
```

## 0004: Window::last_presentation (discussion first)

Branch `gpui-wayland-presentation-time`. Adds a public `Window::last_presentation()` and
`PlatformWindow::last_presentation()`, backed by `wp_presentation` on Wayland. That is a new GPUI
API, so per CONTRIBUTING it went to a discussion in the "Zed GPUI" category first:
https://github.com/zed-industries/zed/discussions/65731 (opened 2026-10-16). Open the PR only after
a staff reply. Open PR #65013
(mmirj) binds `wp_presentation` for frame pacing; if it merges, this patch should reuse its global
binding instead of adding a second one.

Discussion body (posted with minor edits):

```
I'd like a way for a GPUI app to read when the compositor actually put its last frame on
screen. I use it to measure key-to-photon latency and to check frame pacing in tests (we gate
on key-to-present p95 under one 60 Hz interval). Today an app can only time its own draw
calls, which says nothing about when the frame reached the display.

What I have working on Wayland: a `PresentationTime { presented: Instant, refresh: Option<Duration> }`
returned by `Window::last_presentation()`, filled from `wp_presentation_feedback.presented`, and
`None` on platforms that don't measure it. The branch is
https://github.com/FormalSnake/zed/tree/gpui-wayland-presentation-time.

#65013 also binds `wp_presentation`, for pacing. If that lands I'd build on its binding. Would you
take this API, and should it be Wayland only for now or wait for macOS (CVDisplayLink /
`presentedTime`) and Windows (`DXGI_FRAME_STATISTICS`) too?

AI assistance was used
```

## 0005: gpui_windows: Draw the new size in WM_SIZING, before the window takes it

Branch `gpui-windows-wm-sizing`. PR: not opened yet (Zed's three-open-PR limit; the rebased
branch passes `cargo check` and clippy for `x86_64-pc-windows-msvc`, and the after numbers are
from nd's fork at 93d7456).

Issue search: no open issue for resize lag on Windows.

```
## Summary

When you drag a window edge on Windows, GPUI resizes its swap chain in `WM_SIZE` and draws on
the next vsync tick. DWM shows the grown window first and GPUI's frame a composition later, so
the dragged edge shows a strip of the backdrop. In a frame capture of a right-edge drag, 3 to 11
of about 60 frames had the content one drag step (4 px) short of the window edge.

This handles `WM_SIZING`: it works out the client size the proposed window rectangle will get,
resizes and draws that frame before the window takes the size. At worst a frame is clipped by a
window that has not grown yet, which shows nothing missing.

## Testing

Windows 11, RTX 5070 laptop with a 240 Hz panel and a 144 Hz monitor. Same capture of a
right-edge drag: 1 to 3 of 100 frames one step short, down from 3 to 11 of 60. A unit test
covers the rectangle to client size mapping. <rerun on this branch and fill in>.

## Self-review

- [x] I've reviewed my diff for quality, security, reliability, and performance.
- [ ] UI changes follow the [checklist](https://zed.dev/docs/development/ui-checklist). (No UI change.)
- [x] Tests cover the new or changed behavior.

AI assistance was used

Release Notes:

- Fixed a strip of background showing along the dragged edge while resizing a window on Windows.
```

## 0006: gpui_windows: Tick each window with the vblank of the monitor it is on

Branch `gpui-windows-monitor-vblank`. PR: https://github.com/zed-industries/zed/pull/65729
(opened 2026-10-16). The rebase dropped a conflict with the removed `translate_accelerator`;
the branch passes `cargo check` and clippy (`--lib --tests --features gpui/test-support`) for
`x86_64-pc-windows-msvc` from the macbook (llvm-rc from nixpkgs on PATH for gpui's manifest).
The rates were measured on nd's fork at 93d7456; a run of this branch on Windows is still owed.

Issue search: nothing for mixed refresh monitors on Windows. #35379 (wait for DWM vsync) was
closed unmerged.

Body as posted:

```
## Summary

On Windows every GPUI window is ticked from one `DwmFlush` thread, and DWM's clock runs at the fastest display's rate. I have a 240 Hz laptop panel and a 144 Hz monitor, and a window on the 144 Hz monitor drew at 235 to 252 Hz, so a good share of its frames never reached the screen. I first tried skipping ticks of the DWM clock, which only got that window to 121 to 127 Hz, because 240 Hz ticks don't line up with 144 Hz vblanks.

This adds one thread per DXGI output that waits on `IDXGIOutput::WaitForVBlank` and ticks the windows on that monitor. A window spanning two monitors follows the faster one (measured on the visible frame bounds, so the invisible resize border doesn't count). The `DwmFlush` thread still ticks windows on monitors without their own clock, such as one plugged in after startup, and still handles device loss.

## Testing

Windows 11 on a laptop with an RTX 5070 and the Intel iGPU, a 2560x1600 240 Hz panel and a 2560x1440 144 Hz external monitor, one window on each. Draw rates: 239.8 to 240.2 Hz on the panel and 144.0 to 144.1 Hz on the monitor, swapped when I moved the windows across, and 240 Hz for a window straddling both. I measured this with the same change on an earlier main (93d7456). The rebase onto current main only hit the removed `translate_accelerator`, and the branch builds and passes clippy for `x86_64-pc-windows-msvc`. A unit test covers picking the faster monitor.

## Self-review

- [x] I've reviewed my diff for quality, security, reliability, and performance.
- [ ] UI changes follow the [checklist](https://zed.dev/docs/development/ui-checklist). (No UI change.)
- [x] Tests cover the new or changed behavior.

AI assistance was used

Release Notes:

- Fixed windows on Windows redrawing at the fastest monitor's refresh rate instead of the rate of the monitor they are on.
```

## 0007: gpui_windows: Render grayscale text without grid fitting

Branch `gpui-windows-grayscale-text`. PR: not opened yet. Needs before and after captures next
to an inbox app (Settings) on Windows, since it is a visual change and taste is the reviewers'
call. Issue search: #41230 (blurry text with blurred transparency, closed) is the nearest;
#65664 (blurry text at odd client height) is a different bug.

```
## Summary

GPUI turns ClearType off for translucent windows (Mica, blurred), and the grayscale path then
takes DirectWrite's recommended rendering mode. At 100% scale that grid-fits, which snaps stems
to the pixel grid and looks hard and pixelated next to Windows' own apps. Settings and the other
inbox apps render grayscale text symmetric without grid fitting.

This uses `DWRITE_RENDERING_MODE1_NATURAL_SYMMETRIC` with grid fitting off for grayscale text.
ClearType text is unchanged.

## Testing

Windows 11 at 100% scale, a Mica GPUI window next to Settings. <before/after captures>

## Self-review

- [x] I've reviewed my diff for quality, security, reliability, and performance.
- [x] UI changes follow the [checklist](https://zed.dev/docs/development/ui-checklist).
- [ ] Tests cover the new or changed behavior. (Rendering mode choice; covered by the captures.)

AI assistance was used

Release Notes:

- Improved text rendering in translucent windows on Windows to match native apps.
```

## 0008: gpui_apple: Only nudge the font size of a run that continues the same font

Branch `gpui-apple-ligature-nudge`. PR: https://github.com/zed-industries/zed/pull/65728 (opened
2026-10-16; cla-bot asks for the CLA, then a `@cla-bot check` comment).

Issue search: nothing filed about text width on macOS.

Checked on the macbook: `cargo test -p gpui_apple --features font-kit,runtime_shaders,test-support`
(5 text_system tests pass), clippy clean, and the new test fails on the old logic (152.71 vs
152.85 px at 11 px). The Metal toolchain is not installed here, hence `runtime_shaders`.

Body as posted:

```
## Summary

`layout_line` on macOS sets every other font run at `next_up()` of the font size, so CoreText
can't merge two adjacent runs of the same font and form a ligature across them. It alternates
regardless of font, so a line with a single run gets the nudged size too. SF tracks by its exact
point size, and the nudged line comes out a little narrower than CoreText sets the same string
at the real size: 152.71 px instead of 152.85 px for an 11 px line in the system font. I noticed
it as GPUI text not lining up with AppKit labels in the same font.

Now only a run that continues the previous run's font gets the nudge, since that's the only case
where CoreText would merge them. Single-run lines and runs in different fonts keep their exact
size.

## Testing

Added `test_layout_line_sets_a_lone_run_at_its_exact_size`, which lays out a single-run line at
11, 17 and 22 px in the system font and compares its width with a `CTLine` at the exact size. It
fails on main (152.71 vs 152.85 at 11 px) and passes with this change.
`cargo test -p gpui_apple --features font-kit,runtime_shaders` passes on macOS 27 and clippy is
clean.

## Self-review

- [x] I've reviewed my diff for quality, security, reliability, and performance.
- [ ] UI changes follow the [checklist](https://zed.dev/docs/development/ui-checklist). (No UI change.)
- [x] Tests cover the new or changed behavior.

AI assistance was used

Release Notes:

- N/A
```

## 0009: keep the swapchain through an interactive resize (not upstreamable as is)

Branch `gpui-wgpu-live-resize-swapchain`, stacked on `gpui-wayland-configure-ack`, with nd's
`ND_DEBUG_SWAPCHAIN` hook removed. On g815 it checks clean and its three new tests pass. Kept
out for now:

- It works around a Hyprland behaviour: during a live resize Hyprland samples past the
  `wp_viewport` source crop into the oversized buffer, so the patch adds an extra full-window
  pass (`EdgeExtend`) every resize frame to repeat the edge pixels. That is a compositor quirk
  carried by every client, and other compositors may clamp correctly.
- nd's resize gate showed a 4 to 8 px background strip in 115 of 127 drag frames on g815 with an
  earlier version; the edge-extend fix addresses it, but no clean gate run is recorded.
- There is no measured frame rate. The motivation (each reconfigure idles the device for several
  ms on a weak GPU) comes from a simulated cost, not a real weak GPU.

The oversized surface with a viewport crop is a general idea, and #63448 (slow resize on
Wayland with an Intel iGPU, KWin) is the symptom it targets. Next step: measure Zed's resize
frame rate on e1504g with and without the branch, and if it helps, open a discussion in "Zed
GPUI" linking #63448 before any PR.
