+++
title = "State"
description = "Entities and observation, controlled values, background work, clocks, errors and saved state."
+++

State lives in entities. A view reads it in `render`; a handler changes it and calls `cx.notify()`.

<!-- docs:example hello -->

## Models and observation

A model is an entity with no `render`. A view that shows it observes it, and the observer runs after
the model calls `cx.notify()`:

<!-- docs:snippet examples/hello/tests/state.rs#observe -->

`cx.observe` returns a `Subscription`. Dropping it ends the observation, so a constructor that wants
the observer for the view's whole life calls `.detach()`. The observer gets `&mut Self`, the observed
entity and a `Context<Self>`; it copies what it needs and notifies itself. The test changes the model
from outside the view and reads the result off the test backend:

<!-- docs:snippet examples/hello/tests/state.rs#observe-test -->

Observation is how a child tells its parent that something changed: the shared model is the channel.
A view does not emit typed events to its parent yet; there is no `EventEmitter` or `cx.subscribe` in
`nd-ui`. Native controls have events (`on_click`, `on_change`, `on_submit`), and those reach your
view through `cx.listener`.

## Controlled values

Controls that hold a value (`slider`, `select`, `text_field`, tabs) are controlled. The app owns the
value, passes it in on every render and writes the new one back in the change handler:

<!-- docs:snippet examples/chrome/src/lib.rs#controlled -->

The framework keeps a record of what each native control holds and applies a value only when it
differs. Setting the value the control already shows sends nothing, and events caused by the
framework's own setter calls are dropped, so feeding a value back never produces a second set or an
event loop. If a handler refuses a change by leaving the state alone, the control is set back to the
app's value on the next frame. When the control normalises a value (a slider snapping to its step),
the app receives the normalised value once.

## Background work

There is no `cx.spawn` or executor timer in `nd-ui` yet. Off-thread work goes through
`nd::system::spawn(future, done)`: the future runs off the main thread, and `done` runs on the main
thread with its output and the `App`. `done` may hold entities, so it holds a weak one and updates
the view through it:

<!-- docs:snippet examples/system/src/lib.rs#spawn -->

`WeakEntity::update` returns `None` when the view has closed, and the result is ignored. The same
shape serves every callback that outlives the handler that created it, such as the answer of an alert
or a file dialog:

<!-- docs:snippet examples/chrome/src/lib.rs#delete-alert -->

Call `spawn` from the main thread. In a tier 1 test no host runs the continuations, so call
`nd::system::run_pending(cx)` after the future finishes; the [system example's test](../../../examples/system/tests/tier1.rs) does.

## Clocks and timers

Use `nd::time` for `Instant` and `SystemTime`. On `wasm32` the standard versions compile and then
panic when called, and `nd::time` swaps in browser-safe ones. `nd check` flags `std::time::Instant`,
`std::time::SystemTime` and `std::thread::sleep` in UI code.

## Errors

Return `Result` from your own code and show failures in the UI: a `Err` from a system call is a
value in `done`, as the `run` helper above logs it. A panic is not a UI state. On a panic, a native
crash or a hang the diagnostics layer writes an incident bundle (UI tree, recent inputs, frame
timings, logs) under the app data directory, and `nd diagnose` summarises the bundles. Call
`nd::report_issue("what happened")` to write one on demand; it returns the bundle directory, or
`None` on the web, where only the panic hook to the browser console exists.

## Saved state

`nd_settings::Store<T>` keeps machine state such as window frames, recents or a library: JSON on disk
as `{"version": N, "data": ..}`, written after a short debounce through a temp file and rename, and
flushed on drop. `Store::open(path, version, migrate)` loads it. A missing file gives
`T::default()`. A file that cannot be read or migrated is moved aside as
`<name>.corrupt-<unix time>`, defaults win, and `load_error()` says why. `get()` returns an
`Arc<T>` and `update(|data| ..)` changes it and schedules the write. On the web the same type writes
to `localStorage`.

The store is not re-exported through `nd` and no example in the repository uses it yet, so there is
no snippet here. Add `nd-settings` to your `Cargo.toml` to use it. User-facing options belong in the
settings layer instead; see [settings and config](../guides/settings-and-config.md). The
[source](../../../crates/nd-settings/src/store.rs) is short and documents each call.
