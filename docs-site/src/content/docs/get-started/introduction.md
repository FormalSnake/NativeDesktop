---
title: Introduction
description: What NativeDesktop is, how an app is structured, and the rules the framework holds to.
---

NativeDesktop is a cross-platform desktop framework. You write React 19 in TypeScript and get real
native widgets: GTK4 with libadwaita on Linux, AppKit on macOS, Win32 planned. There is no embedded
browser on the UI path, no DOM, no Electron. Apps that need to show web content get a `<webview>`
widget backed by the platform's own engine (WKWebView on macOS, WebKitGTK on Linux) while the UI
around it stays native.

## Two processes, one protocol

Every app runs as two processes.

A Zig host owns `main()` and the platform's UI loop: GLib's main loop on Linux, `NSApplication.run`
through a thin Swift shell on macOS. It holds the authoritative retained widget tree.

A Bun/TypeScript child runs your React app. Your components never touch a widget directly. React's
reconciler diffs your tree and sends the result over NDP, a length-prefixed JSON-RPC protocol on a
local socket, as one `CommitBatch` per commit.

Because they are separate processes, a JS crash or hang leaves the window standing. The host keeps
answering automation requests and, where wired, restarts the child. A crash inside the native
toolkit is not isolated, which is the same exposure any native app has.

## One tree, two design languages

The widgets your JSX describes are the platform's own classes (`GtkBox`, `AdwHeaderBar`,
`NSButton`, `NSSplitView`), so a single React tree renders in each platform's current design
language: Liquid Glass on macOS, Adwaita on GNOME. Dark mode is automatic on both platforms for
anything without an explicit color override.

Styling follows from that. The `style` prop is not CSS. It covers theme-neutral geometry like
padding, layout, and font size. To reach a platform's actual design language, use `cssClasses`, a
set of named classes borrowed from libadwaita's vocabulary that map onto real AppKit control
properties on macOS and real GTK CSS classes on Linux. See
[Styling & Design Language](/core-concepts/styling-design-language/).

## Shared code with web and React Native

`@nativedesktop/react` declares `react` as a `peerDependency` instead of vendoring a copy, so in a
monorepo a NativeDesktop app hoists the same `react` instance as a web (`react-dom`) app or a React
Native app beside it. That single-instance guarantee is what lets one hooks and logic package be
shared verbatim across all three. Author a hook the normal way with
`import { useState } from "react"`, and NativeDesktop's build rewrites the import to the pinned
`@nativedesktop/react` for you, in a production build and under `bun --hot` alike. Desktop-only UI
lives in `.desktop.tsx` files, the same platform-suffix convention React Native uses for
`.native.tsx`. See [Monorepo & Code Sharing](/get-started/monorepo/).

## Rules the framework holds to

**Native chrome is real.** Sidebars are `NSSplitView` or `AdwOverlaySplitView`, never a styled
`Box` pretending to be one. If a platform cannot honor a widget faithfully, it becomes a stub or an
escape hatch rather than silently downgrading.

**The schema is the single source of truth.** Every widget's props, events, and defaults live in
`schema/widgets.json`. The Zig, TypeScript, and Swift bindings are generated from it, along with the
[Widget Reference](/components/widget-reference/). Hand-written per-widget bindings are banned.

**Agents are a first-class consumer.** Every widget a React tree creates is tracked and answerable
over a JSON-RPC socket from the moment `NATIVE_AUTOMATION=1` is set. A coding agent drives an app
the way a user does. See [Automation-First](/core-concepts/automation-first/).

**No color literals by default.** Dark mode and platform theming come from `cssClasses` and the
system's own style manager. Hardcoding a color through `style.color` or `style.background` is an
explicit override.

**Docs mark what has landed.** Windows, for example, is a designed backend that has not been
implemented, and the docs say so.

**Failures are loud.** An unknown `style` key is rejected at the React renderer with a fix-it
message and rejected again host-side. A bad automation action returns a real JSON-RPC error code.
The same rule governs app code: a render throw or an unhandled rejection is reported rather than
swallowed. See [Error Handling](/core-concepts/error-handling/).

## Next

- [Quick Start](/get-started/quick-start/): run an example app in a few commands.
- [Project Layout](/get-started/project-layout/): where the schema, codegen output, and app code live.
- [Monorepo & Code Sharing](/get-started/monorepo/): share hooks and logic with web and React Native.
- [App Model](/core-concepts/app-model/): how a window and its chrome are built from JSX.
