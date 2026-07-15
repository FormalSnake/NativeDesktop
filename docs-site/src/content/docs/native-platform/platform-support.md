---
title: Platform Support
description: What's landed, what's planned, and how each platform is verified.
---

| Platform | Backend | Status | Verified by |
|---|---|---|---|
| Linux | GTK4 + libadwaita | **Landed** | `ci.yml`'s `linux` job, blocking: `zig build test`, codegen-freshness diff, `zig build`, and a chain of `scripts/headless-*.sh` legs (smoke, `kill -9` crash isolation, M2 through M8) run under `weston --backend=headless` with `GSK_RENDERER=cairo` |
| macOS | AppKit (Swift shell over a GTK-free `libnd.a`) | **Landed** | `mac.yml`'s `macos-appkit` job (non-blocking: `continue-on-error: true`, so it never blocks a merge) builds `libnd.a` + the Swift shell on a stock `macos-latest` runner and drives the counter example headfully (no Screen Recording/TCC dependency); locally, `scripts/mac/mac-m11.sh` and related mac scripts run the fuller native-chrome + notes gate |
| Windows | Win32 + Direct2D/DirectWrite (custom-drawn, UIA-provided widgets) | **Planned, not implemented** | design spec only (`docs/superpowers/specs/2026-07-09-nativedesktop-design.md`, decision D8) |

## Detecting the platform at runtime

`Platform` from `@nativedesktop/react` exposes two independent axes, because they can disagree:

```tsx
import { Platform } from "@nativedesktop/react";

Platform.backend; // "gtk" | "appkit" — the native widget layer actually drawing
Platform.os;      // "macos" | "linux" | "windows" — where the process runs

// Branch on the renderer:
const inset = Platform.select({ gtk: 6, appkit: 8, default: 6 });
if (Platform.backend === "appkit") {
  /* AppKit-specific tweak */
}
```

Branch on `backend` for renderer-specific behavior and on `os` for OS conventions (paths,
keybindings, menu placement). They're separate because the GTK backend also runs on macOS (via
GTK's Quartz `gdk`), so `Platform.os === "macos"` doesn't imply AppKit; `process.platform`
alone can't tell you which widgets are drawing.

`backend` is authoritative from the host: it arrives in the NDP `helloAck` (the core learns it from
each embedder's `nd_set_backend_name` and echoes it back), and the renderer installs it before
your tree mounts, so reading `Platform.backend` inside a component, effect, or handler is always
safe. It reads `"unknown"` only before `render()`'s handshake completes. `os` derives from the Bun
child's own `process.platform`. See [Architecture](/core-concepts/architecture/) for where the
handshake sits in the system.

## Platform-only widgets

A widget's schema entry can declare a `platforms` list (e.g. `["macos"]`) restricting it to specific
OSes. Today's two are `<trayitem>` (a macOS menu-bar extra, `NSStatusItem`) and `<sharebutton>` (the
macOS system share picker, `NSSharingServicePicker`). Neither has a native counterpart on GNOME, so
GTK mounts them as an invisible no-op placeholder rather than refusing to render: a tree that
uses one still builds and runs on Linux, it just shows nothing there.

This is a deliberate, supported pattern for platform-specific polish. Not every widget has to look
identical on every OS, as long as omitting it on other platforms is itself the platform-correct
choice. Gate the surrounding UI with `Platform.os === "macos"` so Linux users don't see empty space
where the widget would have been:

```tsx
{Platform.os === "macos" ? (
  <trayitem iconName="face-smile-symbolic" tooltip="My App" />
) : null}
```

In development (`nd dev`, `ND_DEV=1`), mounting a platform-excluded widget on a platform it doesn't
list logs a one-time console warning (`<trayitem> is macOS-only; it renders nothing on linux — gate
with Platform.os.`), so a missing `Platform.os` gate doesn't fail silently. The warning is skipped
entirely in a production build (`nd build`). See [Overview](/components/overview/#what-each-widget-declaration-carries)
for the schema mechanic and [Menu Bar](/native-platform/menu-bar/#beyond-the-menu-bar) for how
`<trayitem>`'s own dropdown menu is built from `<menu>`/`<menuitem>` children.

## Linux: the reference backend

Linux is where the full gate runs and blocks merges. The GTK4 backend also runs natively on macOS
(via GTK4's Quartz `gdk` backend). It isn't the shipping macOS backend, but it means GTK-side
framework changes can be runtime-verified directly on a Mac before ever touching the Linux box.

## macOS: a thin shell over the shared core

The AppKit backend is a small Swift shell (`swift/Sources/NDShell/`) linking against the same
GTK-free Zig core (`libnd.a`, `zig build libnd -Dbackend=abi`) that Linux's widget-tree and
automation logic run on; only the widget-creation and prop-application arms are platform-specific.
A stock GitHub-hosted `macos-latest` runner can build and drive it headfully without Screen
Recording permission, since the counter/notes drive scripts talk to the automation socket rather
than screen capture. The `mac.yml` job is intentionally non-blocking: a red mac run never blocks a
Linux-gated merge.

Offscreen automation screenshots have a known macOS 26 caveat: `_NSCoreHostingView` only paints via
CoreAnimation when actually composited on screen, so an in-process offscreen render can return blank
`TextInput`/`TextArea` content. `tools/ndshot/` (a small Swift ScreenCaptureKit tool with its own
stable code-signing identity) works around this by capturing the live composited window instead;
see `docs/agents/automation.md` for the one-time Screen Recording grant flow.

## Windows: designed, not built

The Windows backend is fully specified (raw Win32 windowing via `zigwin32`, custom-drawn
Direct2D/DirectWrite widgets each carrying a UIA provider and `AutomationId` from day one) but not
yet implemented. The design doc budgets 2–3× the effort of the other two backends and explicitly
schedules it last, since Windows has no first-class native-toolkit equivalent to GTK4/AppKit that a
thin binding could sit on top of. `tools/package.ts` already reflects this: it accepts `linux` and
`mac` as packaging targets and exits with an explicit error for anything else. Don't rely on any
Windows-specific prop, flag, or command not documented elsewhere on this site.
