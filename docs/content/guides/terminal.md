+++
title = "Terminal"
description = "Show a shell or any byte stream in a window with nd-terminal: TerminalSession, terminal(), theme colors, input, and what runs on each OS."
+++

`nd-terminal` is a terminal widget. libghostty-vt parses the program's output and keeps the
screen and scrollback. A pseudo-terminal or any byte stream carries the bytes, and a GPU
presenter on each backend draws the grid. Your app owns a `TerminalSession`, and
`terminal(id, &session)` shows it.

<!-- docs:example terminal -->

## Add it

Depend on `nd-terminal` directly, or turn on nd's `terminal` feature. The example depends on the
crate directly because Cargo unifies nd's features across a workspace, so the feature would link
libghostty-vt into every example.

To build libghostty-vt, you need Zig 0.16 on `PATH` (the dev shell has it) and network access for
the first build. Its build script compiles Ghostty in debug mode whenever the profile has debug
info, and that build parses output thousands of times slower. Add this to the workspace:

```toml
[profile.dev.package.libghostty-vt-sys]
debug = false
```

## A session

A session is the program plus the terminal state. Create one with:

- `spawn` to run a program on a pseudo-terminal.
- `connect` to use any `Read` and `Write` pair, such as a socket to a remote shell.
- `detached` to use no program at all. You feed output in and read what the terminal answers.

<!-- docs:snippet examples/terminal/src/lib.rs#spawn -->

`Command::shell()` runs the user's login shell. To run something else, use
`Command::new(program)` with `.arg`, `.args`, `.cwd`, and `.env`. `SessionOptions` sets the
initial `cols` and `rows` (80 by 24) and the `scrollback` in lines (10 000). The grid resizes to
fit the element, so these values matter only before the first layout.

The session belongs to the app, not the view. Keep it in your state, and a tab switch keeps its
scrollback and its running program. Dropping the last clone ends the child process. Reads and
writes run on their own threads with a bounded inbox, so a program that floods output or stops
reading input never blocks the UI thread. The session parses at most a fixed budget of output
per frame.

## The element

<!-- docs:snippet examples/terminal/src/lib.rs#element -->

Events arrive on the UI thread:

| Handler | Event | When |
| --- | --- | --- |
| `on_title_changed` | `TitleChanged { title }` | The program set the title (OSC 0 or 2). |
| `on_cwd_changed` | `CwdChanged { cwd }` | OSC 7 |
| `on_exit` | `Exited { status }` | The program exited or the stream ended. |
| `on_bell` | `Bell` | BEL |
| `on_notification` | `Notification { title, body }` | OSC 9 or 777 |
| `on_selection_changed` | `SelectionChanged { has_selection }` | A selection was made or cleared. |
| `on_link` | `LinkActivated { link }` | A link was opened with Cmd-click (Ctrl-click outside macOS). |
| `on_focus_changed` | `FocusChanged` | Focus moved. |

Links are OSC 8 hyperlinks and plain URLs. The element doesn't open them. The example passes the
URL to `open` or `xdg-open`.

Use `.font_size(points)`, `.font_family(name)`, and `.padding(points)` to adjust the drawing.
Without them, the element uses the platform's monospace font and size.

## Input and selection

The session encodes keys itself. That includes the Kitty keyboard protocol when a program asks
for it, bracketed paste, mouse reporting, and focus reports (mode 1004). A mouse press goes to a
program that tracks the mouse; otherwise it starts a selection. Hold Shift to force a selection.
Copy writes the selection to the clipboard. Wheel and trackpad scrolling go to the program when
it tracks the mouse. On the alternate screen with alternate scroll on, scrolling turns into arrow
keys. Otherwise it moves through the scrollback.

On macOS, the view implements `NSTextInputClient`, so input methods compose in it. GPUI backends
use the platform text input.

Choose key bindings with care, because plain Ctrl-T and Ctrl-W belong to the shell. The example
binds Cmd-T and Cmd-W on macOS, and Ctrl-Shift-T and Ctrl-Shift-W elsewhere:

<!-- docs:snippet examples/terminal/src/lib.rs#keys -->

To drive a session from code, `write(bytes)` sends typed input, `paste(text)` pastes,
`screen_text()` returns the visible text, and `scroll_rows(n)` moves the viewport. `title()`,
`cwd()`, `exit_status()`, and `is_closed()` read its state.

## Colors

The 16 ANSI colors are theme tokens: `cx.theme().terminal.ansi`. The hues come from the semantic
tokens: red is `danger`, green is `success`, yellow is `warning`, and blue is `accent`. Magenta
and cyan rotate the accent's hue. Each one is adjusted to be legible as text on the base surface.
Black and white swap between light and dark, as light terminal themes do. The background,
foreground, cursor, and selection also come from the theme, so the terminal follows light mode,
dark mode, and a new accent without any code.

To override them, pass `TerminalColors` to `.colors(..)`. Each field is optional: `background`,
`foreground`, `cursor`, `selection`, and `ansi`. The appearance audit accepts the theme-derived
colors, but an override has to meet contrast on its own.

Box drawing, block elements, braille, and Powerline separators are drawn from geometry, so they
meet at cell edges at any font size. Nerd Font icons come from a bundled Symbols Nerd Font Mono
and shrink to fit their cell instead of being clipped.

## What runs where

| | macOS | Linux | Windows |
| --- | --- | --- | --- |
| Presenter | a Metal layer in its own `NSView` | GPUI renderer | GPUI renderer |
| Pseudo-terminal | yes | `portable-pty`, not run yet | ConPTY through `portable-pty`, not run yet |
| Verified | tier 1, tier 2 goldens, window test, scroll budget | not run | not run |

On macOS, the presenter is its own view with a `CAMetalLayer`, presented with the Core Animation
transaction. The GPUI presenter has run only on macOS. Running it on Linux and Windows, and the
ConPTY path, are open items in [TODO.md](../../plan/TODO.md). Kitty graphics are compiled out.

The scroll budget test scrolls 100 000 lines at 3 and 40 rows per frame. Main-thread time per
frame was 0.6 ms at p95 on AppKit and 4.6 ms on GPUI.
