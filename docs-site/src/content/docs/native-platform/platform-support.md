---
title: Platform Support
description: What's landed, what's planned, and how each platform is verified.
---

| Platform | Backend | Status | Verified by |
|---|---|---|---|
| Linux | GTK4 + libadwaita | **Landed** | `ci.yml`'s `linux` job, blocking: `zig build test`, codegen-freshness diff, `zig build`, and a chain of `scripts/headless-*.sh` legs (smoke, `kill -9` crash isolation, M2 through M8) run under `weston --backend=headless` with `GSK_RENDERER=cairo` |
| macOS | AppKit (Swift shell over a GTK-free `libnd.a`) | **Landed** | `mac.yml`'s `macos-appkit` job (stretch, **non-blocking** — `continue-on-error: true`, never blocks a merge) builds `libnd.a` + the Swift shell on a stock `macos-latest` runner and drives the counter example headfully (no Screen Recording/TCC dependency); locally, `scripts/mac/mac-m11.sh` and related mac scripts run the fuller native-chrome + notes gate |
| Windows | Win32 + Direct2D/DirectWrite (custom-drawn, UIA-provided widgets) | **Planned, not implemented** | design spec only (`docs/superpowers/specs/2026-07-09-nativedesktop-design.md`, decision D8) |

## Linux: the reference backend

Linux is where the full gate runs and blocks merges. The GTK4 backend also runs natively on macOS
(via GTK4's Quartz `gdk` backend) — this isn't the shipping macOS backend, but it means GTK-side
framework changes are runtime-verifiable directly on a Mac before ever touching the Linux box.

## macOS: a thin shell over the shared core

The AppKit backend is a small Swift shell (`swift/Sources/NDShell/`) linking against the same
GTK-free Zig core (`libnd.a`, `zig build libnd -Dbackend=abi`) that Linux's widget-tree and
automation logic run on — only the widget-creation and prop-application arms are platform-specific.
A stock GitHub-hosted `macos-latest` runner can build and drive it headfully with no Screen
Recording permission needed, since the counter/notes drive scripts talk to the automation socket,
not to screen capture. The `mac.yml` job is intentionally non-blocking: a red mac run never blocks a
Linux-gated merge.

Offscreen automation screenshots have a known macOS 26 caveat: `_NSCoreHostingView` only paints via
CoreAnimation when actually composited on screen, so an in-process offscreen render can return blank
`TextInput`/`TextArea` content. `tools/ndshot/` (a small Swift ScreenCaptureKit tool with its own
stable code-signing identity) works around this by capturing the *live composited* window instead —
see `docs/agents/automation.md` for the one-time Screen Recording grant flow.

## Windows: designed, not built

The Windows backend is fully specified (raw Win32 windowing via `zigwin32`, custom-drawn
Direct2D/DirectWrite widgets each carrying a UIA provider and `AutomationId` from day one) but not
yet implemented. The design doc budgets 2–3× the effort of the other two backends and explicitly
schedules it last, since Windows has no first-class native-toolkit equivalent to GTK4/AppKit that a
thin binding could sit on top of. `tools/package.ts` already reflects this: it accepts `linux` and
`mac` as packaging targets and exits with an explicit error for anything else. Don't rely on any
Windows-specific prop, flag, or command not documented elsewhere on this site.
