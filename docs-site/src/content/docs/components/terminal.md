---
title: Terminal
description: The <terminal> widget embeds a real terminal emulator (libghostty-vt) as a native drawing surface on every backend.
---

`<terminal>` embeds a real terminal emulator as a native drawing surface, backed by
`libghostty-vt`, the VT engine behind [Ghostty](https://ghostty.org). It spawns a PTY running your
shell, parses the escape-sequence stream into a cell grid, and draws that grid with the platform's
own text stack: CoreText on macOS, cairo and Pango on GTK. Keystrokes go straight to the PTY.

![The terminal widget running a shell on macOS (AppKit)](../../../assets/screens/appkit/terminal.png)

![The terminal widget running a shell on GNOME (GTK)](../../../assets/screens/gtk/terminal.png)

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

That is a working terminal with no native code in your app. Type at the prompt, run `vim`, get
colors and a cursor.

## Props

| Prop         | Type     | Default   | Notes                                                        |
| ------------ | -------- | --------- | ------------------------------------------------------------ |
| `command`    | string   | `$SHELL`  | Program to run in the PTY. Falls back to `/bin/sh`.          |
| `cwd`        | string   | inherited | Working directory for the spawned program.                  |
| `fontSize`   | number   | `13`      | Point size of the monospace cell font.                      |
| `foreground` | string   | `#cccccc` | Default text color, `#rrggbb`.                               |
| `background` | string   | `#000000` | Default grid color, `#rrggbb`.                               |
| `cols`       | number   | `80`      | Initial columns.                                             |
| `rows`       | number   | `24`      | Initial rows.                                                |

The two color defaults are byte-identical on both backends and do not follow the system theme, so a
terminal stays dark under a light appearance the way Terminal.app and GNOME Console do. A color you
pass wins. On macOS, a terminal that spans its window also tints the window background to its own
background color and picks the titlebar appearance from that color's luminance, so a dark grid gets
no white band above it.

All props are `create`-time. The terminal owns its PTY for the life of the widget, so changing a
prop means remounting with a different `key`. Keystrokes are handled host-side and fed straight to
the PTY, never crossing NDP, which keeps the interactive hot path off the socket.

## How it works

Three layers, split the way Ghostty splits its own core from its app runtime:

- **`libghostty-vt`** is emulation only. It parses the VT stream and maintains the grid, scrollback,
  cursor, colors, and reflow. It does not spawn a PTY, render, or touch OS input.
- **The `ndterm` core** (`src/core/terminal.zig`, GTK-free) owns those three. It runs `forkpty`,
  pumps the PTY into `libghostty-vt` on a reader thread behind a mutex, and exposes a flat C ABI
  (`include/ndterm.h`): `ndterm_open`/`close`/`resize`, `ndterm_write_input(bytes)`, and a
  lock-snapshot render model (`ndterm_render_lock` → `ndterm_cell(x, y)` / `ndterm_cursor` →
  `ndterm_render_unlock`).
- **The surface** is the only per-backend piece: a `GtkDrawingArea` painting cells with cairo on
  Linux, an `NSView` painting with CoreText on macOS. Both inset the grid 6 points from the widget
  edge so column 0 is not cut by the frame, and both call only the `ndterm` C ABI.

`libghostty-vt` ships as a prebuilt static library under `vendor/libghostty-vt/`, built once with
the Zig 0.15 toolchain it targets and linked as a plain C archive, so it stays independent of the
repo's pinned Zig 0.16. Nothing is downloaded at build or run time.

## Remote mode

With `remote`, the widget spawns no PTY. The host process dials a byte-lane TCP server itself and
feeds the terminal from that stream, so raw bytes never cross the app's JS bridge:

```tsx
<terminal remote host="127.0.0.1" port={4618} sessionId={id} ticket={ticket} />
```

`host`, `port`, `sessionId`, and `ticket` are create-time props like everything else on the widget:
to attach with a different ticket or session, remount with a different `key`.

Connection sharing is keyed by `host:port:ticket`. Terminals opened with the same ticket share one
TCP connection and the server assigns each an attach channel. A different ticket always gets its
own connection and reader thread, even to the same endpoint, so a stale connection never blocks a
fresh attach and one terminal's grant never limits another's.

The connection auto-reconnects with backoff and replays its ticket on each attempt. Progress
arrives through `onConnectionState`, where `data.state` indexes `connecting`, `authed`, `attached`,
`reconnecting`, `failed`, `closed` (the `nd_rt_state` order in `include/ndremote.h`). `failed` is
terminal for that connection: the server refused auth or attach, so retrying the same ticket cannot
succeed. Mint a fresh ticket and remount.

## Not implemented yet

The terminal renders the grid and routes keyboard input on both backends. Terminal-initiated events
back to React (title changes, bell, child exit) are raised internally by the core but not yet
surfaced as `onTitle`/`onBell`/`onExit` handlers. Mouse reporting, IME and dead-key composition, and
live font-size changes are also outstanding.
