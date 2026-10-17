+++
title = "Diagnostics"
description = "What nd-diag records in every app without code, and how to read an incident bundle after a panic, crash, or hang."
+++

Every nd app runs [nd-diag](../../../crates/nd-diag/src/lib.rs) from its first frame. You don't need to
enable anything. Logs roll to disk, and a flight recorder keeps the last inputs and frames in
memory. A panic, a native crash, or a main-thread hang writes an incident bundle that you can read
with `nd diagnose`.

<!-- docs:example hello -->

## Where files go

Files go under the app data directory (`ND_APP_DATA_DIR` overrides it):

- `logs/<app>.log`: rolls at 10 MB. nd-diag keeps 7 gzipped files and deletes them after 7 days.
- `diagnostics/<utc timestamp>-<kind>/`: one directory per incident. `kind` is `panic`, `crash`,
  `hang`, or `report`.

`ND_LOG` takes a tracing `EnvFilter`. The default is `info`.

## What a bundle holds

| File | Content |
| --- | --- |
| `panic.txt`, `hang.json`, `crash.json` | The trigger: the message and backtrace, the blocked thread and time, or the crash signal. |
| `minidump.dmp` | Every thread's stack, for crashes and hangs, when the crash helper runs. |
| `log.ndjson` | The last 5000 log events. |
| `tree.json` | The semantic tree of every window (the same schema as the automation `getTree`) and, on macOS, the native view and layer hierarchy. |
| `input.ndjson` | The last 200 inputs. Release builds replace typed text with bullets; `ND_DIAG_KEEP_TEXT=1` keeps it. |
| `frames/` | Per-frame layout diffs of the last 120 frames on macOS; the last presented frame per window on the GPUI backends. |
| `profile.json` | Frame timing histograms against the display interval. |
| `meta.json` | App, version, git sha, OS, and backend. |

macOS doesn't record pixels in production. An AppKit window has no drawable that the app owns, and
`renderInContext` misses glass, vibrancy, and web content. The layout diffs and the native
hierarchy take the place of pixels.

The `hello` test below panics the example on purpose and asserts on the bundle that it leaves.

<!-- docs:snippet examples/hello/tests/incidents.rs#panic-bundle -->

## Panics, crashes and hangs

A panic hook writes the `-panic` bundle, then the process exits with code 101. A helper process
(crash-handler plus minidumper) catches native crashes. While something changes, the app refreshes
a snapshot of the tree, inputs, and log once a second. After a crash, the helper writes the
`-crash` bundle from that snapshot and the minidump. Set `ND_DIAG_CRASH_HANDLER=0` to run without
the helper.

A watchdog thread keeps one ping queued on the main thread. When the ping stays unanswered for 5 s
(`ND_DIAG_HANG_MS` changes the threshold), the watchdog writes the `-hang` bundle with the main
thread's stack. A hang doesn't end the process: the app keeps running, and a log line says when the
main thread answers again. The planned wait-or-relaunch window after 10 s isn't built yet.

For tests, `ND_DEBUG_PANIC=1`, `ND_DEBUG_CRASH=1`, and `ND_DEBUG_HANG_MS=<ms>` trigger each one
after the first frame in the AppKit host. [incidents.rs](../../../examples/hello/tests/incidents.rs) uses
them on macOS.

## Reading a bundle

```sh
nd diagnose --last        # summary of the newest bundle
nd diagnose --list        # bundles, newest first
nd diagnose <id> --json   # machine-readable
```

The summary has what an agent needs before it reproduces a bug: the trigger and top frames, the
semantic tree, the last inputs, the slowest frames, and the log tail. `--app <name>` picks the app
when the current directory doesn't identify it. The test below runs it over a real panic bundle.

<!-- docs:snippet examples/hello/tests/incidents.rs#diagnose -->

## Report an issue from the app

```rust
let bundle: Option<std::path::PathBuf> = nd::report_issue("user report");
```

This writes a `-report` bundle and returns its directory. It returns `None` when diagnostics
aren't running, which is always the case on the web. Wire it to a Help menu item that reveals the
directory. For `reveal_path`, see [system](../guides/system.md).

## Retention

nd-diag keeps the newest 20 bundles, none older than 14 days, and 200 MB in total. Cleanup runs on
a background thread after the first frame.

## The web

On the web, nd-diag installs only the panic hook, which writes to the browser console. A browser
tab has no files, threads, or helper process, so there are no bundles.

## Tracy

The `tracy` feature of nd-diag adds a tracing layer for the Tracy profiler (on demand, localhost
only). Without the feature, none of it is compiled in. For the frame budgets, see
[performance](performance.md).
