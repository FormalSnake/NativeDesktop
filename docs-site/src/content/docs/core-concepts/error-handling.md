---
title: Error Handling
description: Which errors kill the app, which ones it survives, and how to observe both from app code.
---

Two lanes, one rule: render errors belong to React error boundaries, async errors belong to the
framework's error policy. A boundary cannot see a rejected promise, and the policy cannot see a
render throw before React does.

## Defaults

The defaults are identical in dev and prod; only the console text differs.

| Kind | Default | Fatal? |
| --- | --- | --- |
| Unhandled promise rejection | report | no |
| Uncaught sync exception | fatal | yes |
| Render throw with no boundary | fatal | yes, not configurable |
| Render throw caught by a boundary | report | no |
| Recoverable render error | report | no |

An orphaned promise says nothing about heap integrity, so the app keeps running. A sync throw
unwound an arbitrary stack with the heap possibly mid-mutation, so the process exits and the host
paints the crash overlay. A render throw that no boundary caught already unmounted the root: the
window is blank, and the overlay is the correct end state.

Non-fatal errors never touch the crash overlay. They are printed to stderr, reported to the host
(which logs `ND_RUNTIME_ERROR_NONFATAL` without storing the message), and dispatched to
subscribers.

## Observing errors

```ts
import { onUnhandledError } from "@nativedesktop/react";

const off = onUnhandledError((error, context) => {
  // context.kind: "unhandledRejection" | "uncaughtException" |
  //   "renderUncaught" | "renderCaught" | "renderRecoverable"
  // context.fatal: true when the process exits after handlers run
  // context.raw: the original thrown/rejected value, verbatim
  // context.componentStack: present for the three render kinds
  myLogger.report(error, context);
});
```

Handlers run in registration order. A handler that itself throws is caught and logged, never
escalated. Same HMR caveat as `app.onActivate`: a module-top-level registration subscribes a fresh
closure on every hot re-eval, so register from `useMountEffect` or a module that runs once.

## Changing the policy

```ts
import { setUnhandledErrorPolicy } from "@nativedesktop/react";

setUnhandledErrorPolicy({
  unhandledRejection: "fatal",   // die on orphaned rejections
  uncaughtException: "report",   // survive sync throws (not recommended)
  log: false,                    // suppress the framework's stderr line; subscribers still fire
});
```

Precedence: an explicit `setUnhandledErrorPolicy` call beats the `ND_FATAL_REJECTIONS=1`
environment variable (which flips the rejection default to fatal), which beats the built-in
defaults. The two render-phase report kinds and the uncaught render throw are not configurable.

## Error boundaries

Boundaries work the standard React way (`getDerivedStateFromError` / `componentDidCatch`); the
framework additionally reports each caught error as a non-fatal `renderCaught` so it reaches
stderr and `onUnhandledError` instead of vanishing. See `examples/errors/main.tsx` for a working
boundary next to every async failure mode.

## Rate cap

Non-fatal reports share one token bucket: 20 reports per rolling 10 seconds. On exhaustion the
framework logs `[nd] further error reports suppressed (rate limit)` once and drops the console
line and the host report until the window reopens. Subscribers keep firing. Fatal errors always
bypass the cap. The cap exists because surviving errors turns a one-shot death into a possible
per-frame error loop that would starve UI commits.

## Known edge

Throwing a non-object from render (`throw "string"`) logs the error twice on the fatal path: the
dedupe between the render report and the follow-on process-level exception uses a `WeakSet`, which
cannot hold primitives. The process still exits exactly once.
