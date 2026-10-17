# Upstreaming the gpui patches

nd carries nine patches on zed (`crates/nd-gpui/patches`, applied to zed@93d7456 by
`scripts/gpui-fork`). This is where each one went in zed-industries/zed, what was measured, and
what is left. Branches live on [FormalSnake/zed](https://github.com/FormalSnake/zed), rebased on
zed main 2d4c483 (2026-10-16); the local worktree is
`/Volumes/Kyan Drive/dev-cache.noindex/zed-upstream`.

## Zed's rules that apply

From `CONTRIBUTING.md` and `.github/pull_request_template.md` on main:

- CLA signed (done 2026-10-16; all three PRs pass `verification/cla-signed`).
- At most three PRs open at a time. That is why the fixes are grouped by platform.
- Features need a GitHub discussion first; bug fixes go straight to a PR.
- Non-trivial changes need tests; visual changes need before and after screenshots.
- AI policy: no PRs from autonomous agents, and PR text should come from the author. Every body
  states `AI assistance was used`.
- Zed's `AGENTS.md`/`CLAUDE.md`/`.rules` tell AI agents to prepend a two-line review marker to
  `README.md` for the human author to delete. The PRs do not carry it; Codex's review bot flags
  that on each PR.
- Template: Summary, Testing, Self-review, then `Release Notes:`. Titles look like
  `gpui_linux: Fix ...`.

## Where each patch went

| PR | Branch | Commits (patch) |
| --- | --- | --- |
| [#65728](https://github.com/zed-industries/zed/pull/65728) macOS | `gpui-apple-ligature-nudge` | font size nudge only for a run continuing the same font (0008) |
| [#65729](https://github.com/zed-industries/zed/pull/65729) Windows | `gpui-windows-monitor-vblank` | per-monitor vblank (0006), draw in `WM_SIZING` (0005), grayscale text (0007) |
| [#65730](https://github.com/zed-industries/zed/pull/65730) Linux | `gpui-wayland-configure-ack` | configure ack with the frame (0003), drop a superseded deferred configure (fix for Codex's review), X11 per-CRTC refresh (0001), live resize surface in gpui_wgpu and its Wayland use (0009, split in two) |
| [discussion #65731](https://github.com/zed-industries/zed/discussions/65731) | `gpui-wayland-presentation-time` | `Window::last_presentation` (0004), a new API, so discussion first |
| zed [#63313](https://github.com/zed-industries/zed/pull/63313) (not ours) | none | GL only when Vulkan fails (0002) is that PR; drop 0002 when it merges |

0009 changed on the way up: `ND_DEBUG_SWAPCHAIN` is gone, the viewport crop is clamped to the
attached buffer (wp_viewport rejects a source outside it), and the edge pass is documented as
what it is: protection against compositors that sample past the crop.

## Measurements

### Linux interactive resize (0003 and 0009)

`crates/nd-os-linux/tests/zed_resize.rs` (Hyprland, labwc) and `scripts/zed-resize-mutter.py`
(mutter) drag the right border out 240 px and the bottom border 144 px, 120 steps 16 ms apart,
on a GPUI example built from the zed branch that fills its window with one colour and renders
every frame (`ZED_PROBE_BIN`), and capture every presented frame. g815, Core Ultra 9 275HX,
2026-10-17, three runs each, frames rendered per second during the drag:

| Compositor | GPU | main + 0003 | + 0009 |
| --- | --- | --- | --- |
| Hyprland 0.56 | Intel iGPU | 47 to 50 | 59 to 60 |
| Hyprland 0.56 | lavapipe | 60 | 59 to 60 |
| labwc 0.20 (wlroots 0.20) | Intel iGPU | 27 to 28 | 27 to 28 |
| labwc 0.20 | lavapipe | 28 to 29 | 27 to 28 |
| mutter 50.4 | lavapipe | 22 to 24 | 22 to 24 |

No captured frame showed background or foreign pixels at the edges in any of these. With the
edge pass turned off (a local build), Hyprland showed them in 25 to 26 of 34 captured frames per
run (garbage on the iGPU, background on lavapipe); labwc stayed clean, so labwc respects the
crop. labwc paces the client at about 28 fps during a resize whatever the client does. Under
headless mutter the probe never draws with the Intel Vulkan driver, on main too; mutter offers no
capture protocol, so it is frame rate only. KWin (`kwin_wayland --virtual`) and Weston start
headless but give a plain client no virtual input, so no resize could be driven there.

### X11 per-CRTC refresh (0001)

`zed_x11_refresh` runs the probe under Xwayland on a headless Hyprland with a 240 Hz and a 144 Hz
output and moves it between them. So far: on the 240 Hz output the probe renders at 240 fps
with and without 0001 (two runs each). The move to the 144 Hz output failed in that run (the
`movewindowpixel` dispatcher does not parse under Hyprland's Lua config); the test now moves the
window by workspace (`hl.dsp.window.move`), not yet run because g815 went offline at 02:25 on
2026-10-17. The PR says the cross-monitor case is still to be measured.

### Windows (0005, 0006, 0007)

On hardware, 2026-10-14 (nd's Windows lane on g815 booted into Windows, RTX 5070 laptop, 240 Hz
panel and 144 Hz monitor): per-monitor vblank 239.8 to 240.2 and 144.0 to 144.1 Hz, swapped
after a move, 240 across the seam (235 to 252 Hz on both before); resize gate 1 to 3 of 100
frames one drag step short with `WM_SIZING` (3 to 11 of about 60 before, in the VM).

The rebased branch in the g815 Windows VM, 2026-10-16: `cargo test -p gpui_windows` passes
except the two credential tests, which need an interactive logon and fail over ssh. Grayscale
text captures at 100% scale, before and after, are on the fork's `pr-assets` branch and in
#65729. A rough GDI-capture drag check in the VM showed no edge strip with or without
`WM_SIZING`, so it says nothing either way.

## Rerun

On g815 (`scripts/linux-gate sync` first): build the probe from a zed checkout (a GPUI example
that prints `zed-probe: frame <w>x<h>` per render, with `ZED_PROBE_CSD=1` for client-side
resize), then in the nd dev shell
`ZED_PROBE_BIN=<probe> ZED_LABEL=<name> cargo test -p nd-os-linux --test zed_resize -- --ignored --nocapture`,
with `VK_ICD_FILENAMES` at the dev shell's `lvp_icd` or `intel_icd`; for mutter
`dbus-run-session -- python3 scripts/zed-resize-mutter.py <mutter> <probe> <label>` with jeepney on
`PYTHONPATH`. Run it under `~/.cache/nd-build/build.lock` so no build skews the frame rate.

## Left

- Codex review comments still open: #65728 (a BOM run, identical fonts under two ids),
  #65729 (an `unwrap` on the vblank thread spawn, monitor handoff between two caches).
- GPUI never drawing under headless mutter with the Intel Vulkan driver, on main too.
- 0004 waits on a staff reply in #65731 and on #65013 (which also binds `wp_presentation`).
