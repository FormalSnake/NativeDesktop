+++
title = "State"
description = "Entities and observation, controlled values, background work, clocks, errors, and saved state."
+++

State lives in entities. A view reads it in `render`, and a handler changes it and calls
`cx.notify()`.

<!-- docs:example hello -->

## Models and observation

A model is an entity with no `render`. A view that shows it observes it, and the observer runs after
the model calls `cx.notify()`:

<!-- docs:snippet examples/hello/tests/state.rs#observe -->

`cx.observe` returns a `Subscription`. Dropping it ends the observation, so a constructor that
wants the observer for the view's whole life calls `.detach()`. The observer gets `&mut Self`, the
observed entity, and a `Context<Self>`. It copies what it needs and notifies itself. The test
changes the model from outside the view and reads the result off the test backend:

<!-- docs:snippet examples/hello/tests/state.rs#observe-test -->

A child tells its parent that something changed through observation, with the shared model as the
channel. A view can't emit typed events to its parent yet, because `nd-ui` has no `EventEmitter` or
`cx.subscribe`. Native controls have events (`on_click`, `on_change`, `on_submit`), and those reach
your view through `cx.listener`.

## Controlled values

Controls that hold a value (`slider`, `select`, `text_field`, tabs) are controlled. Your app owns
the value, passes it in on every render, and writes the new one back in the change handler:

<!-- docs:snippet examples/chrome/src/lib.rs#controlled -->

The framework keeps a record of what each native control holds and applies a value only when it
differs. Setting the value that the control already shows sends nothing, and the framework drops
events that its own setter calls cause, so feeding a value back never produces a second set or an event
loop. If a handler refuses a change by leaving the state alone, the framework sets the control back
to the app's value on the next frame. When the control normalizes a value (a slider snapping to its
step), the app receives the normalized value once.

## Background work

`nd-ui` has no `cx.spawn` or executor timer yet. Run off-thread work through
`nd::system::spawn(future, done)`. The future runs off the main thread, and `done` runs on the main
thread with the future's output and the `App`. When `done` needs a view, give it a weak entity and
update the view through that:

<!-- docs:snippet examples/system/src/lib.rs#spawn -->

`WeakEntity::update` returns `None` when the view has closed, and the code ignores the result. Use
the same shape for every callback that outlives the handler that created it, such as the answer from
an alert or a file dialog:

<!-- docs:snippet examples/chrome/src/lib.rs#delete-alert -->

Call `spawn` from the main thread. In a tier 1 test, no host runs the continuations, so call
`nd::system::run_pending(cx)` after the future finishes, as the
[system example's test](../../../examples/system/tests/tier1.rs) does.

## Clocks and timers

Use `nd::time` for `Instant` and `SystemTime`. On `wasm32`, the standard versions compile and then
panic when called, and `nd::time` swaps in browser-safe ones. `nd check` flags `std::time::Instant`,
`std::time::SystemTime`, and `std::thread::sleep` in UI code.

## Errors

Return `Result` from your own code and show failures in the UI. An `Err` from a system call is a
value in `done`, and the `run` helper above logs it. A panic isn't a UI state. On a panic, a native
crash, or a hang, the diagnostics layer writes an incident bundle (UI tree, recent inputs, frame
timings, logs) under the app data directory, and `nd diagnose` summarizes the bundles. To write one
on demand, call `nd::report_issue("what happened")`. It returns the bundle directory, or `None` on
the web, where diagnostics install only the panic hook, which writes to the browser console.

## Saved state

`nd_settings::Store<T>` keeps machine state such as window frames, recents, or a library. It stores
JSON on disk as `{"version": N, "data": ..}`, writes it through a temp file and rename after a short
debounce, and flushes it on drop. `Store::open(path, version, migrate)` loads it. A missing file
gives `T::default()`. The store moves a file that it can't read or migrate aside as
`<name>.corrupt-<unix time>`, uses the defaults, and reports why in `load_error()`. `get()` returns
an `Arc<T>`, and `update(|data| ..)` changes it and schedules the write. On the web, the same type
writes to `localStorage`.

`nd` doesn't re-export the store, and no example in the repository uses it yet, so this page has no
snippet. To use it, add `nd-settings` to your `Cargo.toml`. User-facing options belong in the
settings layer instead. See [settings and config](../guides/settings-and-config.md). The
[source](../../../crates/nd-settings/src/store.rs) is short and documents each call.
