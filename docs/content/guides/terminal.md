+++
title = "Terminal"
description = "Show a shell or any byte stream in a window with nd-terminal: TerminalSession, terminal(), theme colours, input, and what runs on which OS."
+++

`nd-terminal` is a terminal widget. libghostty-vt parses the program's output and keeps the
screen and scrollback, a pseudo-terminal or any byte stream carries the bytes, and a GPU
presenter per backend draws the grid. The app owns a `TerminalSession`; `terminal(id, &session)`
shows it.

<!-- docs:example terminal -->

## Add it

Depend on `nd-terminal` directly, or turn on nd's `terminal` feature. The example uses the crate
directly, because Cargo unifies nd's features across a workspace and the feature would link
libghostty-vt into every example.

Building libghostty-vt needs Zig 0.16 on `PATH` (the dev shell has it) and network access for
the first build. Its build script compiles Ghostty in debug mode whenever the profile has debug
info, and that parses output thousands of times slower. Add this to the workspace:

```toml
[profile.dev.package.libghostty-vt-sys]
debug = false
```

## A session

A session is the program plus the terminal state. Make one with `spawn` for a program on a
pseudo-terminal, `connect` for any `Read` and `Write` pair such as a socket to a remote shell,
or `detached` for none, where you feed output in and read what the terminal answers.

<!-- docs:snippet examples/terminal/src/lib.rs#spawn -->

`Command::shell()` runs the user's login shell. `Command::new(program)` with `.arg`, `.args`,
`.cwd` and `.env` runs something else. `SessionOptions` sets the initial `cols` and `rows`
(80 by 24) and the `scrollback` in lines (10 000). The grid resizes to the element, so those
only matter before the first layout.

The session belongs to the app, not the view. Hold it in your state and a tab switch keeps its
scrollback and its running program. Dropping the last clone ends the child. Reading and writing
run on their own threads with a bounded inbox, so a program that floods output or stops reading
input never blocks the UI thread, and the session parses at most a budget of output per frame.

## The element

<!-- docs:snippet examples/terminal/src/lib.rs#element -->

Events arrive on the UI thread:

| Handler | Event | When |
| --- | --- | --- |
| `on_title_changed` | `TitleChanged { title }` | the program set the title (OSC 0 or 2) |
| `on_cwd_changed` | `CwdChanged { cwd }` | OSC 7 |
| `on_exit` | `Exited { status }` | the program exited or the stream ended |
| `on_bell` | `Bell` | BEL |
| `on_notification` | `Notification { title, body }` | OSC 9 or 777 |
| `on_selection_changed` | `SelectionChanged { has_selection }` | selection made or cleared |
| `on_link` | `LinkActivated { link }` | a link opened with Cmd-click (Ctrl-click off macOS) |
| `on_focus_changed` | `FocusChanged` | focus moved |

Links are OSC 8 hyperlinks and plain URLs. The element does not open them. The example hands the
URL to `open` or `xdg-open`.

`.font_size(points)`, `.font_family(name)` and `.padding(points)` adjust the drawing. Without
them the platform's monospace font and size are used.

## Input and selection

The session encodes keys itself, including the Kitty keyboard protocol when a program asks for it,
bracketed paste, mouse reporting and focus reports (mode 1004). A mouse press goes to a program
that tracks the mouse and starts a selection otherwise; Shift forces a selection. Copy writes the
selection to the clipboard. Wheel and trackpad scrolling go to the program when it tracks the
mouse, turn into arrow keys on the alternate screen with alternate scroll on, and move through
the scrollback otherwise.

On macOS the view implements `NSTextInputClient`, so input methods compose in it. GPUI backends
use the platform text input.

Key bindings need care: plain Ctrl-T and Ctrl-W belong to the shell. The example binds Cmd-T and
Cmd-W on macOS and Ctrl-Shift-T and Ctrl-Shift-W elsewhere:

<!-- docs:snippet examples/terminal/src/lib.rs#keys -->

For code that drives a session directly, `write(bytes)` sends typed input, `paste(text)` pastes,
`screen_text()` returns the visible text, `scroll_rows(n)` moves the viewport and `title()`,
`cwd()`, `exit_status()` and `is_closed()` read state.

## Colours

The 16 ANSI colours are theme tokens: `cx.theme().terminal.ansi`. The hues are the semantic
tokens (red is `danger`, green `success`, yellow `warning`, blue `accent`; magenta and cyan turn
the accent's hue), each made legible as text on the base surface. Black and white swap ends
between light and dark, as light terminal themes do. The background, foreground, cursor and
selection come from the theme too, so the terminal follows light and dark and a new accent with
no code.

To override, pass `TerminalColors` to `.colors(..)`. Each field is optional: `background`,
`foreground`, `cursor`, `selection` and `ansi`. The appearance audit accepts the
theme-derived colours; an override has to meet contrast on its own.

Box drawing, block elements, braille and Powerline separators are drawn from geometry so they
meet at cell edges at any font size. Nerd Font icons come from a bundled Symbols Nerd Font Mono
and shrink to fit their cell instead of being clipped.

## What runs where

| | macOS | Linux | Windows |
| --- | --- | --- | --- |
| Presenter | a Metal layer in its own `NSView` | GPUI renderer | GPUI renderer |
| Pseudo-terminal | yes | `portable-pty`, not run yet | ConPTY through `portable-pty`, not run yet |
| Verified | tier 1, tier 2 goldens, window test, scroll budget | not run | not run |

On macOS the presenter is its own view with a `CAMetalLayer`, presented with the Core Animation
transaction. The GPUI presenter has run on macOS only. The Linux and Windows runs of it and the
ConPTY path are open in [TODO.md](../../plan/TODO.md). Kitty graphics are compiled out.

The scroll budget test scrolls 100 000 lines at 3 and 40 rows per frame. Main-thread time per
frame was p95 0.6 ms on AppKit and 4.6 ms on GPUI.
