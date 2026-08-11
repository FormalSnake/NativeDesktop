// Pure-logic tests for the notification data correlation map and the
// app.isActive() activation mirror — no host process; the ndp is a stub
// whose request() just hands back ids.
import { test, expect, beforeEach } from "bun:test";
import { setHmrState, type HmrState } from "./hmr.ts";
import { notifications, app, dispatchSystemEvent } from "./system.ts";

let nextId = 0;
beforeEach(() => {
  globalThis.__nd_system_events = undefined;
  globalThis.__nd_notification_data = undefined;
  globalThis.__nd_app_active = undefined;
  nextId = 0;
  const fakeNdp = { request: async () => `n${++nextId}` };
  setHmrState({ ndp: fakeNdp, root: null, reconciler: { updateContainer: () => {} }, bootCount: 1 } as unknown as HmrState);
});

test("show() stores data; click echoes it once and deletes the entry", async () => {
  const id = await notifications.show({ title: "t", data: { run: 42 } });
  const seen: unknown[] = [];
  notifications.onClick((e) => seen.push([e.id, e.data]));
  dispatchSystemEvent("notification.click", { id });
  expect(seen).toEqual([[id, { run: 42 }]]);
  // A second click for the same id has no stored payload anymore.
  dispatchSystemEvent("notification.click", { id });
  expect(seen[1]).toEqual([id, undefined]);
});

test("a notification shown without data clicks through with data absent", async () => {
  const id = await notifications.show({ title: "plain" });
  let got: { id: string; data?: unknown } | undefined;
  notifications.onClick((e) => (got = e));
  dispatchSystemEvent("notification.click", { id });
  expect(got).toEqual({ id });
  expect(got && "data" in got).toBe(false);
});

test("map is FIFO-capped at 128", async () => {
  const first = await notifications.show({ title: "first", data: "oldest" });
  for (let i = 0; i < 128; i++) await notifications.show({ title: `n${i}`, data: i });
  expect(globalThis.__nd_notification_data!.size).toBe(128);
  expect(globalThis.__nd_notification_data!.has(first)).toBe(false);
});

test("isActive defaults true, then mirrors activate/deactivate transitions", () => {
  expect(app.isActive()).toBe(true);
  dispatchSystemEvent("app.deactivate", {});
  expect(app.isActive()).toBe(false);
  dispatchSystemEvent("app.activate", {});
  expect(app.isActive()).toBe(true);
});

test("a handler reading isActive() inside onActivate sees the new state", () => {
  dispatchSystemEvent("app.deactivate", {});
  let insideHandler: boolean | undefined;
  app.onActivate(() => (insideHandler = app.isActive()));
  dispatchSystemEvent("app.activate", {});
  expect(insideHandler).toBe(true);
});
