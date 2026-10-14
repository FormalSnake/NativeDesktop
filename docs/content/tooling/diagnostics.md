+++
title = "Diagnostics"
description = "What nd-diag records in every app without code, and how to read an incident bundle after a panic, crash or hang."
+++

Every nd app runs [nd-diag](../../../crates/nd-diag/src/lib.rs) from its first frame. There is nothing to
enable: logs roll to disk, a flight recorder keeps the last inputs and frames in memory, and a
panic, native crash or main-thread hang writes an incident bundle you can read with
`nd diagnose`.

<!-- docs:example hello -->

## Where files go

Under the app data dir (`ND_APP_DATA_DIR` overrides it):

- `logs/<app>.log`: rolls at 10 MB, 7 files kept, gzipped, deleted after 7 days.
- `diagnostics/<utc timestamp>-<kind>/`: one directory per incident. `kind` is `panic`, `crash`,
  `hang` or `report`.

`ND_LOG` takes a tracing `EnvFilter` (default `info`).

## What a bundle holds

| File | Content |
| --- | --- |
| `panic.txt`, `hang.json`, `crash.json` | The trigger: message and backtrace, the blocked thread and time, or the crash signal. |
| `minidump.dmp` | Every thread's stack, for crashes and hangs when the crash helper runs. |
| `log.ndjson` | The last 5000 log events. |
| `tree.json` | The semantic tree of every window (the same schema as the automation `getTree`) and, on macOS, the native view and layer hierarchy. |
| `input.ndjson` | The last 200 inputs. Typed text is replaced by bullets in release builds; `ND_DIAG_KEEP_TEXT=1` keeps it. |
| `frames/` | Per-frame layout diffs of the last 120 frames on macOS; the last presented frame per window on the GPUI backends. |
| `profile.json` | Frame timing histograms against the display interval. |
| `meta.json` | App, version, git sha, OS, backend. |

macOS does not record pixels in production: an AppKit window has no drawable the app owns, and
`renderInContext` misses glass, vibrancy and web content. The layout diffs and the native
hierarchy stand in for them.

The `hello` test below panics the example on purpose and asserts the bundle it leaves.

<!-- docs:snippet examples/hello/tests/incidents.rs#panic-bundle -->

## Panics, crashes and hangs

A panic hook writes the `-panic` bundle, then the process exits with code 101. A native crash is
caught by a helper process (crash-handler plus minidumper): the app refreshes a snapshot of the tree,
inputs and log once a second while something changes, and after a crash the helper writes the
`-crash` bundle from that snapshot plus the minidump. Set `ND_DIAG_CRASH_HANDLER=0` to run without
the helper.

A watchdog thread keeps one ping queued on the main thread. When it stays unanswered for 5 s
(`ND_DIAG_HANG_MS` changes the threshold) the `-hang` bundle is written with the main thread's
stack. The app keeps running; a hang does not end the process, and a log line says when the main
thread answers again. The planned wait-or-relaunch window after 10 s is not built.

For tests, `ND_DEBUG_PANIC=1`, `ND_DEBUG_CRASH=1` and `ND_DEBUG_HANG_MS=<ms>` trigger each one
after the first frame in the AppKit host. [incidents.rs](../../../examples/hello/tests/incidents.rs) uses
them on macOS.

## Reading a bundle

```sh
nd diagnose --last        # summary of the newest bundle
nd diagnose --list        # bundles, newest first
nd diagnose <id> --json   # machine-readable
```

The summary is what an agent needs before reproducing a bug: the trigger and top frames, the
semantic tree, the last inputs, slowest frames and the log tail. `--app <name>` picks the app when
the current directory does not say. The test below runs it over a real panic bundle.

<!-- docs:snippet examples/hello/tests/incidents.rs#diagnose -->

## Report an issue from the app

```rust
let bundle: Option<std::path::PathBuf> = nd::report_issue("user report");
```

This writes a `-report` bundle and returns its directory. It returns `None` when diagnostics are
not running, which is always the case on the web. Wire it to a Help menu item that reveals the
directory; [system](../guides/system.md) covers `reveal_path`.

## Retention

Newest 20 bundles, none older than 14 days, 200 MB in total. Cleanup runs on a background thread
after the first frame.

## The web

Only the panic hook is installed, and it writes to the browser console. Files, threads and a
helper process do not exist in a tab, so there are no bundles.

## Tracy

The `tracy` feature of nd-diag adds a tracing layer for the Tracy profiler (on demand, localhost
only). Nothing is compiled in without it. See [performance](performance.md) for the frame budgets.
