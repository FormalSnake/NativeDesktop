---
title: Terminal
description: The <terminal> widget embeds a real terminal emulator (libghostty-vt) as a native drawing surface on every backend.
---

`<terminal>` embeds a **real terminal emulator** — the same VT engine that powers
[Ghostty](https://ghostty.org), `libghostty-vt` — as a native drawing surface. It spawns a PTY
running your shell, parses the escape-sequence stream into a cell grid, and draws that grid with the
platform's own text stack: **CoreText on macOS**, **cairo/Pango on GTK**. Keystrokes are captured by
the surface and written straight to the PTY.

```tsx
import { render } from "@nativedesktop/react";

function App() {
  return (
    <window title="Terminal" defaultWidth={720} defaultHeight={460}>
      <terminal cols={80} rows={24} fontSize={13} />
    </window>
  );
}

await render(<App />);
```

That is a complete, working terminal — a shell prompt you can type into, run `vim`, see colors and
the cursor — with no native code in your app.

## Props

| Prop       | Type     | Default   | Notes                                                        |
| ---------- | -------- | --------- | ------------------------------------------------------------ |
| `command`  | string   | `$SHELL`  | Program to run in the PTY. Falls back to `/bin/sh`.          |
| `cwd`      | string   | inherited | Working directory for the spawned program.                  |
| `fontSize` | number   | `13`      | Point size of the monospace cell font.                      |
| `cols`     | number   | `80`      | Initial columns.                                             |
| `rows`     | number   | `24`      | Initial rows.                                                |

All props are `create`-time — the terminal owns its PTY for the life of the widget, so changing them
means remounting (give the widget a different `key`). Keystrokes are handled **host-side** in the
surface and fed directly to the PTY; they never cross the NDP protocol, which keeps the interactive
hot path off the socket entirely.

## How it works — a shared core, a surface per backend

The design mirrors Ghostty's own split of a shared terminal **core** from a per-platform **app
runtime**, which maps cleanly onto NativeDesktop's two backends:

- **`libghostty-vt`** is terminal *emulation only* — it parses the VT stream and maintains the grid,
  scrollback, cursor, colors, and reflow. It does **not** spawn a PTY, render, or touch OS input.
- **The `ndterm` core** (`src/core/terminal.zig`, GTK-free) owns those three: it runs `forkpty`,
  pumps the PTY into `libghostty-vt` on a reader thread behind a mutex, and exposes a tiny C ABI
  (`include/ndterm.h`) that hides all of the emulator's complexity behind a flat, cross-language
  surface — `ndterm_open`/`close`/`resize`, `ndterm_write_input(bytes)`, and a lock-snapshot render
  model (`ndterm_render_lock` → `ndterm_cell(x, y)` / `ndterm_cursor` → `ndterm_render_unlock`).
- **The surface** is the only per-backend piece: a `GtkDrawingArea` painting cells with cairo on
  Linux, an `NSView` painting with CoreText on macOS. Both call *only* the `ndterm` C ABI — neither
  touches `libghostty-vt` directly.

This is the same **native escape-hatch** pattern the widget system describes as its last rung: a
widget that owns a custom-drawn native subtree instead of composing from primitives. The terminal is
its first real inhabitant.

## The engine is vendored, not fetched

`libghostty-vt` ships as a prebuilt static library under `vendor/libghostty-vt/` (built once with the
Zig 0.15 toolchain it targets, then linked as a plain C archive so it is independent of the repo's
pinned Zig 0.16). The GTK host links it into every artifact that compiles the terminal surface; the
macOS shell links it after `libnd` so its `ghostty_*` symbols resolve. Nothing is downloaded at build
or run time.

## Current scope

The first cut renders the grid and routes keyboard input on both backends. Not yet wired:
terminal-initiated **events** back to React (title changes, bell, child-exit) — the core already
raises them internally; surfacing them as `onTitle`/`onBell`/`onExit` handlers is a small follow-up.
Mouse reporting, IME/dead-key composition, and live font-size changes are also future work.
