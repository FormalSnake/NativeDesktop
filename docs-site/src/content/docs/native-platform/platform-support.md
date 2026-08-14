---
title: Platform Support
description: What's landed, what's planned, and how each platform is verified.
---

| Platform | Backend | Status | Verified by |
|---|---|---|---|
| Linux | GTK4 + libadwaita | **Landed** | `ci.yml`'s `linux` job, blocking: `zig build test`, codegen-freshness diff, `zig build`, and a chain of `scripts/headless-*.sh` legs (smoke, `kill -9` crash isolation, M2 through M8) run under `weston --backend=headless` with `GSK_RENDERER=cairo` |
| macOS | AppKit (Swift shell over a GTK-free `libnd.a`) | **Landed** | `mac.yml`'s `macos-appkit` job (non-blocking: `continue-on-error: true`, so it never blocks a merge) builds `libnd.a` + the Swift shell on a stock `macos-latest` runner and drives the counter example headfully (no Screen Recording/TCC dependency); locally, `scripts/mac/mac-m11.sh` and related mac scripts run the fuller native-chrome + notes gate |
| Windows | Win32 + Direct2D/DirectWrite (custom-drawn, UIA-provided widgets) | **Planned, not implemented** | nothing; the backend is designed but unbuilt |

## Detecting the platform at runtime

`Platform` from `@nativedesktop/react` exposes two independent axes, because they can disagree:

```tsx
import { Platform } from "@nativedesktop/react";

Platform.backend; // "gtk" | "appkit": the native widget layer actually drawing
Platform.os;      // "macos" | "linux" | "windows": where the process runs

// Branch on the renderer:
const inset = Platform.select({ gtk: 6, appkit: 8, default: 6 });
if (Platform.backend === "appkit") {
  /* AppKit-specific tweak */
}
```

Branch on `backend` for renderer-specific behavior and on `os` for OS conventions (paths,
keybindings, menu placement). They are separate because the GTK backend also runs on macOS through
GTK's Quartz `gdk`, so `Platform.os === "macos"` does not imply AppKit and `process.platform` alone
cannot tell you which widgets are drawing.

`backend` is authoritative from the host: it arrives in the NDP `helloAck` (the core learns it from
each embedder's `nd_set_backend_name` and echoes it back), and the renderer installs it before
your tree mounts, so reading `Platform.backend` inside a component, effect, or handler is always
safe. It reads `"unknown"` only before `render()`'s handshake completes. `os` derives from the Bun
child's own `process.platform`. See [Architecture](/core-concepts/architecture/) for where the
handshake sits in the system.

## Platform-only widgets

A widget's schema entry can declare a `platforms` list (for example `["macos"]`) restricting it to
specific operating systems. Today's two are `<trayitem>` (a macOS menu-bar extra, `NSStatusItem`)
and `<sharebutton>` (the macOS system share picker, `NSSharingServicePicker`). Neither has a native
counterpart on GNOME, so GTK mounts them as an invisible no-op placeholder. A tree that uses one
still builds and runs on Linux; it just shows nothing there.

This is a supported pattern for platform-specific polish. Gate the surrounding UI with
`Platform.os === "macos"` so Linux users do not see empty space where the widget would have been:

```tsx
{Platform.os === "macos" ? (
  <trayitem iconName="face-smile-symbolic" tooltip="My App" />
) : null}
```

In development (`nd dev`, `ND_DEV=1`), mounting a platform-excluded widget on a platform it doesn't
list logs a one-time console warning (`<trayitem> is macOS-only; it renders nothing on linux. Gate
it with Platform.os.`), so a missing `Platform.os` gate doesn't fail silently. The warning is skipped
entirely in a production build (`nd build`). See [Overview](/components/overview/#what-a-widget-declaration-carries)
for the schema mechanic and [Menu Bar](/native-platform/menu-bar/#beyond-the-menu-bar) for how
`<trayitem>`'s own dropdown menu is built from `<menu>`/`<menuitem>` children.

## Linux: the reference backend

Linux is where the full gate runs and blocks merges. The GTK4 backend also runs natively on macOS
through GTK4's Quartz `gdk` backend. That is not the shipping macOS backend, but it means GTK-side
framework changes can be runtime-verified on a Mac before touching the Linux box.

## macOS: a thin shell over the shared core

The AppKit backend is a small Swift shell (`swift/Sources/NDShell/`) linking the same GTK-free Zig
core (`libnd.a`, `zig build libnd -Dbackend=abi`) that Linux's widget-tree and automation logic run
on. Only the widget-creation and prop-application arms are platform-specific. A stock GitHub-hosted
`macos-latest` runner builds and drives it headfully without Screen Recording permission, since the
drive scripts talk to the automation socket rather than screen capture. The `mac.yml` job is
non-blocking, so a red mac run never blocks a Linux-gated merge.

Offscreen automation screenshots have a macOS 26 caveat: `_NSCoreHostingView` only paints via
CoreAnimation when composited on screen, so an in-process offscreen render can return blank
`TextInput`/`TextArea` content. `tools/ndshot/`, a small Swift ScreenCaptureKit tool with its own
stable code-signing identity, captures the live composited window instead. See
[Screenshots on macOS (ndshot)](/automation-testing/automation-socket/#screenshots-on-macos-ndshot)
for the one-time Screen Recording grant flow.

## Windows: designed, not built

The Windows backend is specified (raw Win32 windowing via `zigwin32`, custom-drawn
Direct2D/DirectWrite widgets each carrying a UIA provider and `AutomationId`) but not implemented.
It is scheduled last because Windows has no first-class native toolkit comparable to GTK4 or AppKit
that a thin binding could sit on, which puts it at roughly two to three times the effort of either
existing backend. [`nd package`](/packaging/) reflects this: it accepts `mac` and `linux` and exits
with an explicit error for anything else. Do not rely on any Windows-specific prop, flag, or command
not documented elsewhere on this site.
