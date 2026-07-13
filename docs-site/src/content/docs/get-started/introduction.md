---
title: Introduction
description: What NativeDesktop is, why it exists, and the principles it holds to.
---

NativeDesktop is a cross-platform desktop framework where you write **React 19 in TypeScript** and
get **real native widgets** back — GTK4/libadwaita on Linux today, AppKit on macOS today, Win32 on
Windows planned. There is no embedded browser on the UI path: no DOM, no WebView, no Electron.

## Two processes, one protocol

Every NativeDesktop app is two processes:

- A **Zig host** owns `main()` and the platform's native UI loop (GLib's main loop on Linux,
  `NSApplication.run` via a thin Swift shell on macOS). It holds the authoritative, retained widget
  tree.
- A **Bun/TypeScript child** runs your React app. Your components never touch a widget directly —
  React's reconciler diffs your tree and sends the result over **NDP** (a length-prefixed JSON-RPC
  protocol over a local socket) as one `CommitBatch` per commit.

This split means a JS crash or hang doesn't take the window down: the host stays up, keeps
answering automation requests, and (where wired) can restart the child. A native-toolkit crash is
the one failure mode NativeDesktop doesn't try to isolate — that's the same failure mode any native
app has.

## One tree, two design languages

Because the widgets your JSX describes *are* the platform's own widget classes — `GtkBox`,
`AdwHeaderBar`, `NSButton`, `NSSplitView` — a single React tree automatically renders in each
platform's current design language: Liquid Glass on macOS, Adwaita on GNOME. You don't maintain two
UIs or a facsimile layer that approximates either one. Dark mode is automatic on both platforms
for anything that isn't given an explicit color override.

Styling reflects this: the `style` prop is deliberately **not CSS** — it's theme-neutral geometry
(padding, layout, font size). Reaching a platform's actual design language is `cssClasses`, a set of
named classes (borrowed from libadwaita's vocabulary) that map onto real AppKit control properties
on macOS and real GTK CSS classes on Linux. See
[Styling & Design Language](/core-concepts/styling-design-language/).

## Shares code with web and React Native

`@nativedesktop/react` declares `react` as a `peerDependency`, not a vendored copy — so in a
monorepo, a NativeDesktop app hoists the *same* `react` instance as a web (`react-dom`) app or a
React Native app living alongside it. That single-instance guarantee is what lets a hooks/logic
package be shared verbatim across all three: author a hook the normal way,
`import { useState } from "react"`, and NativeDesktop's build pipeline rewrites that import to the
pinned `@nativedesktop/react` automatically, both for a production build and for `bun --hot` dev.
Desktop-only UI stays visually separated in `.desktop.tsx` files, the same platform-suffix
convention React Native uses for `.native.tsx`. See
[Monorepo & Code Sharing](/get-started/monorepo/) for the full mechanics.

## Principles

- **Native chrome must be real.** Sidebars are `NSSplitView`/`AdwOverlaySplitView`, not a styled
  `Box` pretending to be one. If a platform can't honor a widget faithfully, it's a stub or an
  escape hatch — never a silent downgrade.
- **The schema is the single source of truth.** Every widget's props, events, and defaults live in
  `schema/widgets.json`; Zig, TypeScript, and Swift bindings — and the
  [Widget Reference](/components/widget-reference/) itself — are generated from it. Hand-written
  per-widget bindings are banned.
- **Automation-first, not automation-bolted-on.** Every widget a React tree creates is tracked and
  answerable over a JSON-RPC socket from the moment `NATIVE_AUTOMATION=1` is set — a coding agent
  drives an app the same way a user would. See [Automation-First](/core-concepts/automation-first/).
- **No color literals by default.** Dark mode and platform theming come from `cssClasses` and the
  system's own style manager, not hardcoded `style.color`/`style.background`. A hardcoded color is
  an explicit, deliberate override, not the default path.
- **Honest status over aspirational docs.** Every doc in this framework — including this site —
  marks what's landed versus planned. Windows support, for example, is a designed backend that has
  not been implemented yet; that's stated plainly rather than implied.
- **Fail loudly, never silently.** An unknown `style` key is rejected at the React renderer with a
  fix-it message and defensively rejected again host-side. A bad automation action returns a real
  JSON-RPC error code, not a swallowed no-op.

## Where to go next

- [Quick Start](/get-started/quick-start/) — run an example app in a few commands.
- [Project Layout](/get-started/project-layout/) — where the schema, codegen output, and app code live.
- [Monorepo & Code Sharing](/get-started/monorepo/) — share hooks/logic with web and React Native.
- [App Model](/core-concepts/app-model/) — how a window and its chrome are built from JSX.
