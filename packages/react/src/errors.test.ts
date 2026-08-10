// Pure-policy tests: decide() precedence, subscribe/unsubscribe dispatch,
// and the non-fatal rate bucket. No process.exit paths are exercised.
import { test, expect, beforeEach } from "bun:test";
import {
  decide,
  onUnhandledError,
  setUnhandledErrorPolicy,
  reportRenderError,
  admitReport,
  type NdErrorContext,
} from "./errors.ts";

beforeEach(() => {
  globalThis.__nd_errors = undefined;
  delete process.env.ND_FATAL_REJECTIONS;
});

// --- decide() ---------------------------------------------------------------

test("defaults: rejection reports, exception is fatal", () => {
  expect(decide("unhandledRejection", {})).toBe("report");
  expect(decide("uncaughtException", {})).toBe("fatal");
});

test("render kinds are not configurable", () => {
  const p = { unhandledRejection: "fatal", uncaughtException: "report" } as const;
  expect(decide("renderUncaught", p)).toBe("fatal");
  expect(decide("renderCaught", p)).toBe("report");
  expect(decide("renderRecoverable", p)).toBe("report");
});

test("explicit policy overrides the defaults", () => {
  expect(decide("unhandledRejection", { unhandledRejection: "fatal" })).toBe("fatal");
  expect(decide("uncaughtException", { uncaughtException: "report" })).toBe("report");
});

test("ND_FATAL_REJECTIONS=1 flips the rejection default; explicit policy still wins", () => {
  process.env.ND_FATAL_REJECTIONS = "1";
  expect(decide("unhandledRejection", {})).toBe("fatal");
  expect(decide("unhandledRejection", { unhandledRejection: "report" })).toBe("report");
});

// --- onUnhandledError -------------------------------------------------------

test("subscribers fire in registration order with kind/fatal/raw context", () => {
  setUnhandledErrorPolicy({ log: false });
  const seen: string[] = [];
  let ctx: NdErrorContext | undefined;
  onUnhandledError(() => seen.push("first"));
  onUnhandledError((_e, c) => {
    seen.push("second");
    ctx = c;
  });
  reportRenderError(new Error("boom"), "renderCaught", "in App");
  expect(seen).toEqual(["first", "second"]);
  expect(ctx?.kind).toBe("renderCaught");
  expect(ctx?.fatal).toBe(false);
  expect(ctx?.componentStack).toBe("in App");
  expect((ctx?.raw as Error).message).toBe("boom");
});

test("unsubscribe stops delivery", () => {
  setUnhandledErrorPolicy({ log: false });
  let calls = 0;
  const off = onUnhandledError(() => {
    calls++;
  });
  reportRenderError(new Error("one"), "renderCaught");
  off();
  reportRenderError(new Error("two"), "renderCaught");
  expect(calls).toBe(1);
});

test("a throwing handler never breaks the next one", () => {
  setUnhandledErrorPolicy({ log: false });
  let reached = false;
  onUnhandledError(() => {
    throw new Error("handler bug");
  });
  onUnhandledError(() => {
    reached = true;
  });
  reportRenderError(new Error("boom"), "renderRecoverable");
  expect(reached).toBe(true);
});

test("non-Error raw values are wrapped for the handler, kept verbatim in raw", () => {
  setUnhandledErrorPolicy({ log: false });
  let got: { e: Error; raw: unknown } | undefined;
  onUnhandledError((e, c) => {
    got = { e, raw: c.raw };
  });
  reportRenderError("plain string", "renderCaught");
  expect(got?.e).toBeInstanceOf(Error);
  expect(got?.e.message).toBe("plain string");
  expect(got?.raw).toBe("plain string");
});

// --- rate bucket ------------------------------------------------------------

test("bucket admits 20 reports per window, then suppresses", () => {
  for (let i = 0; i < 20; i++) expect(admitReport()).toBe(true);
  expect(admitReport()).toBe(false);
  expect(admitReport()).toBe(false);
});

test("bucket reopens once old stamps fall out of the window", () => {
  for (let i = 0; i < 20; i++) admitReport();
  expect(admitReport()).toBe(false);
  // Age every stamp past the 10s window; the next report must be admitted.
  const s = globalThis.__nd_errors!;
  s.stamps = s.stamps.map((t) => t - 11_000);
  expect(admitReport()).toBe(true);
});

test("suppressed reports skip console and wire but still reach subscribers", () => {
  setUnhandledErrorPolicy({ log: false });
  let calls = 0;
  onUnhandledError(() => {
    calls++;
  });
  for (let i = 0; i < 25; i++) reportRenderError(new Error(`e${i}`), "renderCaught");
  expect(calls).toBe(25);
  expect(globalThis.__nd_errors!.stamps.length).toBe(20);
});
