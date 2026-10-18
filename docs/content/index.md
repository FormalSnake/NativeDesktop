+++
title = "NativeDesktop"
description = "One Rust view tree that renders as native AppKit controls on macOS and through GPUI on Linux, Windows, and the web."
+++

NativeDesktop is a toolkit for desktop apps written in Rust. You describe a window as a tree of
views and elements, and each platform's backend turns that tree into the controls its OS draws.

The counter below is all of [hello](../../examples/hello/src/lib.rs): a struct with a `render`
method and a button. The same code runs on every target:

- macOS: AppKit. The toolbar is an `NSToolbar`, the button an `NSButton`, a list an `NSTableView`.
  The framework adds Taffy layout and a retained tree on top of them.
- Linux: GPUI draws the tree with a libadwaita look. Colors come from the theme tokens and, if one
  exists, a matugen palette that reloads while the app runs.
- Windows: the same GPUI backend with a Fluent skin. It runs on Windows hardware, and parts of the
  Fluent skin are still missing.
- Web: plain GPUI compiled to WebAssembly. `nd serve --target web` serves an app without changes.

Your app never names a backend. `scaffold()`, `toolbar()`, and `button()` say what the window
contains, and the backend decides how that looks on its OS. When a platform has no native version
of something, the framework draws it from the same theme tokens.

<!-- docs:example hello -->

## Built for agents

The framework is designed so that a coding agent can build a good app on the first pass and prove
that it works. `nd new` writes a project with a `CLAUDE.md`, a tier 1 test and an appearance audit.
`nd check` fails on literal colors, text below WCAG contrast, `unwrap`, and framework lints.
`nd test` runs the same view with no window. The [agent pages](agents/index.md) list the patterns
to copy and the mistakes to avoid. They're written for people too; nothing in them is agent-only.

## Where to go next

1. [Getting started](getting-started.md): install the `nd` CLI, then create, run, and test an app.
2. [Programming model](concepts/programming-model.md): views, entities and the retained tree.
3. [Elements](concepts/elements.md), [state](concepts/state.md) and
   [actions and keys](concepts/actions-and-keys.md): the vocabulary you use every day.
4. [Components](components/index.md) and the [examples](examples.md): what ships and how it looks.
