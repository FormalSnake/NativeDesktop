---
title: Example Apps
description: Every example app in the repository, captured live on both backends — AppKit on macOS and GTK/libadwaita on GNOME.
---

Each example in [`examples/`](https://github.com/FormalSnake/NativeDesktop/tree/main/examples) is a
complete runnable app (`ND_SCRIPT=examples/<name>/main.tsx`), and each doubles as the acceptance
surface for a framework capability. The screenshots below are captured from the real hosts: the
AppKit shell on macOS and the GTK host under GNOME, both rendering the same unchanged app tree.

## Counter

The hello-world app: a click counter plus `Suspense` and `useTransition` running under a ticking
re-render. [Source](https://github.com/FormalSnake/NativeDesktop/tree/main/examples/counter)

![The counter example on macOS (AppKit)](../../../assets/screens/appkit/counter.png)

![The counter example on GNOME (GTK)](../../../assets/screens/gtk/counter.png)

## Tasks

The smallest real app: a single-pane task list showing the menu-bar and toolbar chrome machinery
scaled down to one window.
[Source](https://github.com/FormalSnake/NativeDesktop/tree/main/examples/tasks)

![The tasks example on macOS (AppKit)](../../../assets/screens/appkit/tasks.png)

![The tasks example on GNOME (GTK)](../../../assets/screens/gtk/tasks.png)

## Notes

A three-pane note-taking app (folders, list, editor) with native chrome, a menu bar, search, and
pinning — the framework-suitability stress test.
[Source](https://github.com/FormalSnake/NativeDesktop/tree/main/examples/notes)

![The notes example on macOS (AppKit)](../../../assets/screens/appkit/notes.png)

![The notes example on GNOME (GTK)](../../../assets/screens/gtk/notes.png)

## Settings

A two-pane preferences window built from the boxed-list widgets: real `AdwPreferencesGroup` rows on
GTK, grouped form rows on macOS.
[Source](https://github.com/FormalSnake/NativeDesktop/tree/main/examples/settings)

![The settings example on macOS (AppKit)](../../../assets/screens/appkit/settings.png)

![The settings example on GNOME (GTK)](../../../assets/screens/gtk/settings.png)

## Gallery

Every widget in one tabbed window — the acceptance surface the drive scripts assert against.
[Source](https://github.com/FormalSnake/NativeDesktop/tree/main/examples/gallery)

![The widget gallery example on macOS (AppKit)](../../../assets/screens/appkit/gallery.png)

![The widget gallery example on GNOME (GTK)](../../../assets/screens/gtk/gallery.png)

## SourceTree

The `<sourcetree>` sidebar widget: sections, a three-level chain, captions, badges, and per-row
actions. [Source](https://github.com/FormalSnake/NativeDesktop/tree/main/examples/sourcetree)

![The sourcetree example on macOS (AppKit)](../../../assets/screens/appkit/sourcetree.png)

![The sourcetree example on GNOME (GTK)](../../../assets/screens/gtk/sourcetree.png)

## Panes

Terminal-style split panes over `@nativedesktop/panes`, with the layout persisted through
`createStore`. [Source](https://github.com/FormalSnake/NativeDesktop/tree/main/examples/panes)

![The panes example split into three panes on macOS (AppKit)](../../../assets/screens/appkit/panes.png)

![The panes example split into three panes on GNOME (GTK)](../../../assets/screens/gtk/panes.png)

## Terminal

A Ghostty-style tabbed terminal: every tab is its own `<window tabGroup>` root running an
independent shell over a real PTY.
[Source](https://github.com/FormalSnake/NativeDesktop/tree/main/examples/terminal)

![The terminal example running a shell on macOS (AppKit)](../../../assets/screens/appkit/terminal.png)

![The terminal example running a shell on GNOME (GTK)](../../../assets/screens/gtk/terminal.png)

## Browser

A small Min-style browser with native system tabs — one `<webview>` per tab window, popups and
downloads routed through app events.
[Source](https://github.com/FormalSnake/NativeDesktop/tree/main/examples/browser)

![The browser example with a loaded page on macOS (AppKit)](../../../assets/screens/appkit/browser.png)

![The browser example with a loaded page on GNOME (GTK)](../../../assets/screens/gtk/browser.png)

## Dialogs

Every dialog surface in one window: app-level `dialog.*`, per-window `showAlert` / `openFile` /
`saveFile`, and the dialog-scripting hooks automation uses.
[Source](https://github.com/FormalSnake/NativeDesktop/tree/main/examples/dialogs)

![The dialogs example with a native alert open on macOS (AppKit)](../../../assets/screens/appkit/dialogs.png)

![The dialogs example with a native alert open on GNOME (GTK)](../../../assets/screens/gtk/dialogs.png)

## Command Palette

A controlled Cmd-K palette used as a directory picker: the app owns the query and recomputes the
result list per keystroke.
[Source](https://github.com/FormalSnake/NativeDesktop/tree/main/examples/command-palette)

![The command palette example with the palette open on macOS (AppKit)](../../../assets/screens/appkit/command-palette.png)

![The command palette example with the palette open on GNOME (GTK)](../../../assets/screens/gtk/command-palette.png)

## Multi-Window

Two windows driven by one React tree, with a live `<webview>` that portals between them without
reloading. [Source](https://github.com/FormalSnake/NativeDesktop/tree/main/examples/multiwindow)

![The multiwindow example on macOS (AppKit)](../../../assets/screens/appkit/multiwindow.png)

## Gestures

The input-synthesis probe: sliders, tables, checkboxes, and text fields laid out as deterministic
targets for the `pointer` / `drag` / `keys` automation RPCs.
[Source](https://github.com/FormalSnake/NativeDesktop/tree/main/examples/gestures)

![The gestures example on macOS (AppKit)](../../../assets/screens/appkit/gestures.png)

![The gestures example on GNOME (GTK)](../../../assets/screens/gtk/gestures.png)

## Errors

The survivable error policy: caught render throws, report-and-survive rejections, and the fatal
crash overlay. [Source](https://github.com/FormalSnake/NativeDesktop/tree/main/examples/errors)

![The errors example with a caught boundary error on macOS (AppKit)](../../../assets/screens/appkit/errors.png)

![The errors example with a caught boundary error on GNOME (GTK)](../../../assets/screens/gtk/errors.png)

## Inspector

The HIG design-gap batch: an inspector split pane, a prominent toolbar button, and edge-to-edge
content. [Source](https://github.com/FormalSnake/NativeDesktop/tree/main/examples/inspector)

![The inspector example on macOS (AppKit)](../../../assets/screens/appkit/inspector.png)

![The inspector example on GNOME (GTK)](../../../assets/screens/gtk/inspector.png)
