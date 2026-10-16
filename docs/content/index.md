+++
title = "NativeDesktop"
description = "One Rust view tree that renders as native AppKit controls on macOS and through GPUI on Linux, Windows and the web."
+++

NativeDesktop is a toolkit for desktop apps written in Rust. You describe a window as a tree of
views and elements; each platform's backend turns that tree into what the OS draws for itself.

<!-- docs:example hello -->

The counter above is the whole of [hello](../../examples/hello/src/lib.rs): one struct, one
`render`, one button. The same code runs on every target:

- macOS: AppKit. The toolbar is an `NSToolbar`, the button an `NSButton`, a list an `NSTableView`.
  The framework adds layout (Taffy) and a retained tree on top of them.
- Linux: GPUI draws the tree with a libadwaita look. Colours come from the theme tokens and, when
  present, a matugen palette that reloads while the app runs.
- Windows: the same GPUI backend with a Fluent skin. The code is written and has not run on a
  Windows machine yet.
- Web: plain GPUI compiled to WebAssembly. `nd serve --target web` serves an app unchanged.

An app never names a backend. `scaffold()`, `toolbar()` and `button()` say what the window is, and
the backend decides how that looks on its OS. Where a platform has no native version of something,
the framework draws it from the same theme tokens.

## Built for agents

The framework is shaped so that a coding agent produces a good app on the first pass and can prove
it works. `nd new` writes a project with a `CLAUDE.md`, a tier 1 test and an appearance audit.
`nd check` fails on literal colours, text under WCAG contrast, `unwrap` and framework lints, and
`nd test` runs the same view with no window. The [agent pages](agents/index.md) list the patterns
to copy and the mistakes to avoid. People use the same pages; nothing in them is agent-only.

## Where to go next

1. [Getting started](getting-started.md): install the `nd` CLI, create an app, run it, test it.
2. [Programming model](concepts/programming-model.md): views, entities and the retained tree.
3. [Elements](concepts/elements.md), [state](concepts/state.md) and
   [actions and keys](concepts/actions-and-keys.md): the vocabulary you write every day.
4. [Components](components/index.md) and the [examples](examples.md): what ships and how it looks.
