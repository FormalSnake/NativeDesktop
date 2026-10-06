import { test, expect, beforeAll } from "bun:test";
import { createSignal, For, Show } from "solid-js";
import { Batch, NodeRegistry, setSession, type Op } from "@nativedesktop/core";
import { render, nextCommit, Portal, createPool } from "./renderer.ts";
import { eventForHandler } from "@nativedesktop/core";

const commits: Op[][] = [];
const registry = new NodeRegistry();

beforeAll(() => {
  const batch = new Batch();
  setSession({
    ndp: {} as never,
    registry,
    batch,
    commit() {
      const ops = batch.drain();
      if (ops.length) commits.push(ops);
    },
    flush() {},
  });
});

const settle = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

/// Mounts `code` in place of the previous test's tree and answers the ops
/// of the mount alone.
async function mount(code: () => unknown): Promise<Op[]> {
  globalThis.__nd_solid_mounted?.dispose();
  globalThis.__nd_solid_mounted = undefined;
  await settle();
  commits.length = 0;
  const done = nextCommit();
  await render(code as never);
  await done;
  return commits.flat();
}

/// Ops every commit `fn`'s writes produced, none when nothing changed.
async function next(fn: () => void): Promise<Op[]> {
  commits.length = 0;
  fn();
  await settle();
  return commits.flat();
}

test("a fresh tree mounts as creates carrying their final props, then appends", async () => {
  const [n] = createSignal(3);
  const ops = await mount(() => (
    <window title="t">
      <box orientation="vertical" style={{ padding: 4 }}>
        <label text={`n ${n()}`} />
        <label>
          a {n()}
        </label>
      </box>
    </window>
  ));
  expect(ops.map((o) => o.op)).toEqual(["create", "create", "create", "append", "create", "append", "append"]);
  expect(ops[0]).toMatchObject({ op: "create", widget: "Window", props: { title: "t" } });
  expect(ops[1]).toMatchObject({ widget: "Box", props: { orientation: "vertical", style: { padding: 4 } } });
  expect(ops[2]).toMatchObject({ widget: "Label", props: { text: "n 3" } });
  expect(ops[4]).toMatchObject({ widget: "Label", props: { text: "a 3" } });
});

test("a signal write ships one update or setText per node, and handlers route events", async () => {
  const [n, setN] = createSignal(0);
  let clicked = 0;
  let btnId = 0;
  await mount(() => (
    <window>
      <button ref={(b) => (btnId = b.id)} label={`b ${n()}`} enabled={n() < 5} onClick={() => clicked++} />
      <label>{n()}</label>
    </window>
  ));
  registry.get(btnId)!.handlers[eventForHandler("button", "onClick")!]!();
  expect(clicked).toBe(1);
  const ops = await next(() => setN(1));
  expect(ops).toHaveLength(2);
  expect(ops).toContainEqual({ op: "update", id: btnId, props: { label: "b 1" } });
  expect(ops).toContainEqual({ op: "setText", id: expect.any(Number), text: "1" });
});

test("a dropped prop resets: null, style to {}, cssClasses to []", async () => {
  const [on, setOn] = createSignal(true);
  let id = 0;
  await mount(() => (
    <window>
      <box
        ref={(b) => (id = b.id)}
        tooltip={on() ? "tip" : undefined}
        style={on() ? { padding: 2 } : undefined}
        cssClasses={on() ? ["card"] : undefined}
      />
    </window>
  ));
  expect(await next(() => setOn(false))).toEqual([{ op: "update", id, props: { tooltip: null, style: {}, cssClasses: [] } }]);
});

test("an object prop with unchanged contents emits nothing", async () => {
  const [n, setN] = createSignal(0);
  await mount(() => (
    <window>
      <box style={{ padding: n() > 10 ? 1 : 2 }} />
    </window>
  ));
  expect(await next(() => setN(1))).toEqual([]);
});

test("a keyed reorder moves widgets without recreating them", async () => {
  const [items, setItems] = createSignal(["a", "b", "c"]);
  const ops0 = await mount(() => (
    <window>
      <box>
        <For each={items()}>{(s) => <label text={s} />}</For>
      </box>
    </window>
  ));
  const ids = ops0.filter((o) => o.op === "create" && o.widget === "Label").map((o) => (o as { id: number }).id);
  const ops = await next(() => setItems(["c", "a", "b"]));
  expect(ops.some((o) => o.op === "create" || o.op === "remove")).toBe(false);
  expect(ops).toContainEqual({ op: "insertBefore", parent: expect.any(Number), child: ids[2]!, before: ids[0]! });
});

test("an unmounted subtree is removed once and comes back with fresh ids", async () => {
  const [show, setShow] = createSignal(true);
  const ops0 = await mount(() => (
    <window>
      <box>
        <Show when={show()}>
          <box>
            <label text="x" />
          </box>
        </Show>
      </box>
    </window>
  ));
  const inner = (ops0[2] as { id: number }).id;
  expect(await next(() => setShow(false))).toEqual([{ op: "remove", id: inner }]);
  expect(registry.get(inner)).toBeUndefined();
  const back = await next(() => setShow(true));
  expect(back.filter((o) => o.op === "create").length).toBe(2);
  expect(back.some((o) => (o as { id?: number }).id === inner)).toBe(false);
});

test("portal children mount detached in the pool, side by side, and leave with their owner", async () => {
  const pool = createPool();
  const [open, setOpen] = createSignal(true);
  const ops = await mount(() => (
    <window>
      <Show when={open()}>
        <Portal pool={pool}>
          <webview testID="a" />
        </Portal>
      </Show>
      <Portal pool={pool}>
        <webview testID="b" />
      </Portal>
    </window>
  ));
  const views = ops.filter((o) => o.op === "create" && o.widget === "WebView") as { id: number }[];
  expect(views).toHaveLength(2);
  const attached = new Set(ops.filter((o) => o.op === "append").map((o) => (o as { child: number }).child));
  expect(views.some((v) => attached.has(v.id))).toBe(false);
  expect(await next(() => setOpen(false))).toEqual([{ op: "remove", id: views[0]!.id }]);
});
