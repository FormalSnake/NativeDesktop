// Async-error policy: decides, per error kind, whether the process reports
// and keeps running or reports and exits. Render-phase errors belong to
// error boundaries (the renderer reports them here through
// reportRenderError); this module owns the process-level
// `unhandledRejection` / `uncaughtException` handlers. All state lives on
// globalThis (same idiom as session.ts) so a `bun --hot` re-eval neither
// double-installs process listeners nor resets subscriptions mid-session.

import { getSession, isHot } from "./session.ts";

export type NdErrorKind =
  | "unhandledRejection"
  | "uncaughtException"
  | "renderUncaught"
  | "renderCaught";

export interface NdErrorContext {
  kind: NdErrorKind;
  /** True when the process will exit after handlers run. */
  fatal: boolean;
  /** Original thrown/rejected value, verbatim (may not be an Error). */
  raw: unknown;
}

export type NdErrorHandler = (error: Error, context: NdErrorContext) => void;

export interface UnhandledErrorPolicy {
  unhandledRejection?: "report" | "fatal";
  uncaughtException?: "report" | "fatal";
  /** false suppresses the framework's own stderr line (app ships its own
   *  logger). Subscribers still fire. Default true. */
  log?: boolean;
}

// Rate cap for non-fatal reports: surviving turns a one-shot death into a
// possible per-frame error loop that would saturate the NDP outbox and
// starve commits. Fatal reports always bypass the cap.
const RATE_MAX = 20;
const RATE_WINDOW_MS = 10_000;

interface ErrorsState {
  policy: UnhandledErrorPolicy;
  handlers: Set<NdErrorHandler>;
  /** Timestamps of admitted non-fatal reports inside the rolling window. */
  stamps: number[];
  suppressedLogged: boolean;
  hintShown: boolean;
}

declare global {
  // eslint-disable-next-line no-var
  var __nd_errors: ErrorsState | undefined;
  // eslint-disable-next-line no-var
  var __nd_errors_installed: boolean | undefined;
}

function state(): ErrorsState {
  if (!globalThis.__nd_errors) {
    globalThis.__nd_errors = {
      policy: {},
      handlers: new Set(),
      stamps: [],
      suppressedLogged: false,
      hintShown: false,
    };
  }
  return globalThis.__nd_errors;
}

/** Pure resolver, exported for tests. Precedence: explicit policy >
 *  ND_FATAL_REJECTIONS=1 env > built-in defaults. */
export function decide(kind: NdErrorKind, p: UnhandledErrorPolicy): "report" | "fatal" {
  switch (kind) {
    case "renderUncaught":
      // No boundary caught it and the tree's reactivity has halted. Not configurable.
      return "fatal";
    case "renderCaught":
      return "report";
    case "uncaughtException":
      // A sync throw unwound an arbitrary stack; the heap may be mid-mutation.
      return p.uncaughtException ?? "fatal";
    case "unhandledRejection":
      return p.unhandledRejection ?? (process.env.ND_FATAL_REJECTIONS === "1" ? "fatal" : "report");
  }
}

/** Subscribe to every reported error. Returns an unsubscribe fn. Handlers
 *  run in registration order; a handler that itself throws is caught and
 *  logged, never escalated. Same HMR caveat as system.ts's app.on*: a
 *  module-top-level call re-subscribes a fresh closure each hot re-eval, so
 *  register from onSettled or a module that runs once. */
export function onUnhandledError(handler: NdErrorHandler): () => void {
  const handlers = state().handlers;
  handlers.add(handler);
  return () => handlers.delete(handler);
}

/** App-entry policy override. Merges over the current policy. */
export function setUnhandledErrorPolicy(p: UnhandledErrorPolicy): void {
  const s = state();
  s.policy = { ...s.policy, ...p };
}

const KIND_LABEL: Record<NdErrorKind, string> = {
  unhandledRejection: "unhandled rejection",
  uncaughtException: "uncaught exception",
  renderUncaught: "render error",
  renderCaught: "render error (caught by boundary)",
};

function toError(raw: unknown): Error {
  return raw instanceof Error ? raw : new Error(String(raw));
}

/** True when a non-fatal report still fits the rolling window; exported for
 *  tests only. Logs the suppression notice once per exhaustion. */
export function admitReport(): boolean {
  const s = state();
  const now = Date.now();
  while (s.stamps.length && now - s.stamps[0]! > RATE_WINDOW_MS) s.stamps.shift();
  if (s.stamps.length >= RATE_MAX) {
    if (!s.suppressedLogged) {
      s.suppressedLogged = true;
      console.error("[nd] further error reports suppressed (rate limit)");
    }
    return false;
  }
  s.suppressedLogged = false;
  s.stamps.push(now);
  return true;
}

function report(raw: unknown, kind: NdErrorKind, fatal: boolean): void {
  const s = state();
  const error = toError(raw);
  // The cap drops the console line and the wire frame, never the subscriber
  // dispatch (in-process, cheap, and the app's own handler must not go blind).
  if (fatal || admitReport()) {
    if (s.policy.log !== false) {
      let line = `[nd] ${KIND_LABEL[kind]}: ${error.message}\n${error.stack ?? ""}`;
      if (!fatal && isHot() && !s.hintShown) {
        s.hintShown = true;
        line += "\n[nd] the app kept running. Register onUnhandledError() from @nativedesktop/react to handle these.";
      }
      console.error(line);
    }
    // No-op before render() connects, same pattern as system.ts's call().
    getSession()?.ndp.sendRuntimeError(error.message, error.stack ?? "", fatal);
  }
  const context: NdErrorContext = { kind, fatal, raw };
  for (const handler of s.handlers) {
    try {
      handler(error, context);
    } catch (handlerErr) {
      console.error(`[nd] onUnhandledError handler threw: ${String(handlerErr)}`);
    }
  }
}

/** Called by a renderer's own error boundaries. Not public API. */
export function reportRenderError(raw: unknown, kind: "renderUncaught" | "renderCaught"): void {
  report(raw, kind, kind === "renderUncaught");
}

/** Installs the process-level handlers once per process; render() calls this
 *  before Ndp.connect() (the wire send no-ops until connected). Guarded on a
 *  globalThis flag so a `bun --hot` re-eval never accumulates listeners. */
export function installErrorHandlers(): void {
  if (globalThis.__nd_errors_installed) return;
  globalThis.__nd_errors_installed = true;
  process.on("unhandledRejection", (reason) => {
    const fatal = decide("unhandledRejection", state().policy) === "fatal";
    report(reason, "unhandledRejection", fatal);
    if (fatal) process.exit(1);
  });
  process.on("uncaughtException", (err) => {
    const fatal = decide("uncaughtException", state().policy) === "fatal";
    report(err, "uncaughtException", fatal);
    if (fatal) process.exit(1);
  });
}
