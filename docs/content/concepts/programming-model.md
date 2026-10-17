+++
title = "Programming model"
description = "Views, entities, and contexts, what render returns, and how the retained tree reaches a backend."
+++

An app is a set of entities. Some of them are views: they implement `Render` and return an element
tree. When an entity changes, it calls `cx.notify()`, and the framework runs `render` again for that
view only. Everything runs in one process on the main thread, so there's no IPC layer to cross.

<!-- docs:example hello -->

## Views and render

<!-- docs:snippet examples/hello/src/lib.rs#counter -->

`Counter` is a plain struct. `impl Render` gives it one method, `render(&mut self, window, cx)`,
which returns `impl IntoElement`. `render` describes how the window should look now. It doesn't
change anything on screen. Keep it cheap and free of side effects, because the framework calls it
whenever the view is notified.

`cx` is a `Context<Counter>`. It dereferences to `App`, so everything an `App` can do (open a
window, read the theme, set a global) works through it, and it adds what is specific to this
entity:

- `cx.notify()` marks this view dirty, and the next frame re-renders it.
- `cx.listener(|this, event, window, cx| ..)` turns a closure over `&mut Counter` into the handler
  an element stores. The framework updates the entity when it delivers the event.
- `cx.observe(&other, |this, other, cx| ..)` runs when `other` notifies. See [state](state.md).
- `cx.weak_entity()` returns a handle that a callback can hold without keeping the view alive.

## Entities

`cx.new(|cx| value)` builds an `Entity<T>`, a shared handle to a `T` that the app owns. Read an
entity with `entity.read(cx)` and change it with `entity.update(cx, |value, cx| ..)`. An `update`
leases the value, so reading or updating the same entity from inside its own `update` panics. Views
are entities, and so are models, which hold data and have no `render`. A `WeakEntity` returns
`None` from `update` once the entity is gone, which is why callbacks use it.

<!-- docs:snippet examples/hello/src/lib.rs#open -->

`open_window` builds the root view with a closure and returns a `WindowHandle<Counter>`.
`handle.id()` is the window id, and `handle.root()` is the root entity. An entity isn't bound to a
window, so a view can move to another window and keep its state.

## Invalidation is per view

`cx.notify()` runs `render` again for that one view. A child that's itself an `Entity` is a
boundary: the parent and the child each render only when they're notified, so a keystroke in a
text editor doesn't re-render the list next to it. A change costs as much as the dirty view, not
the whole window.

After `render`, the framework compares the new element description with the retained tree and sends
the backend only what changed: create, insert, move, remove, update, and, for retained backends, a
frame. Event handlers live on the retained node, and the framework swaps them without a backend
call, so a new closure on every render costs nothing. [Elements](elements.md) explains how a child
is matched to its previous self.

## The backend seam

The framework talks to a platform through one `Backend` trait. The AppKit backend, the GPUI
backend, and the test backend implement it, and your app never sees which one it has.

- Each window gets one transaction per frame: `begin`, the ops in a fixed order (creates, structure
  with parents first, updates, frames, focus, releases), and then `commit`. On AppKit that is one
  `CATransaction` with implicit animations off.
- The backend holds one strong reference per node and drops it on `release`.
- Every backend uses Taffy for layout. On AppKit, the framework runs Taffy and pushes frames to the
  native views. On GPUI, Taffy runs inside GPUI's element tree.
- Everything happens on the main thread.

Native controls never call into your code while the framework is applying changes. AppKit fires
delegate methods inside setters and during layout, and an app entity can be mid-update at that
moment. A native control pushes its events onto a per-window queue instead, and the queue drains
after the transaction, outside every entity lease. That's why a handler can safely change any
entity.

## Tests use the same tree

`TestApp` is an `App` over a `TestBackend` that records the ops and exposes the semantic tree. The
counter's test opens the real `hello::open` and clicks the button by id. For more, see the
[hello tutorial](../getting-started.md#the-hello-app) and the [testing page](../tooling/testing.md).
