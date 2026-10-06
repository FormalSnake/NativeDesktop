import { test, expect, beforeAll } from "bun:test";
import { createSignal, Errored, For, Show } from "solid-js";
import { Batch, NodeRegistry, onUnhandledError, setSession, setUnhandledErrorPolicy, type Op } from "@nativedesktop/core";
import { render, nextCommit, Portal, createPool, Activity } from "./renderer.ts";
import { defineNativeComponent, type NativeComponentRef } from "./native-component.ts";
import { eventForHandler } from "@nativedesktop/core";

const commits: Op[][] = [];
const registry = new NodeRegistry();
const widgetCommands: { id: number; command: string; arg: unknown }[] = [];

beforeAll(() => {
  const batch = new Batch();
  setSession({
    ndp: {
      sendWidgetCommand(id: number, command: string, arg: unknown) {
        widgetCommands.push({ id, command, arg });
      },
      sendRuntimeError() {},
    } as never,
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

test("an Activity hides its widgets in place and shows the same ones again", async () => {
  const [mode, setMode] = createSignal<"visible" | "hidden">("hidden");
  const ops = await mount(() => (
    <window>
      <Activity mode={mode()}>
        <webview testID="page" />
      </Activity>
    </window>
  ));
  const view = ops.find((o) => o.op === "create" && o.widget === "WebView") as { id: number };
  const at = ops.indexOf(view as Op);
  expect(ops.slice(at + 1)).toContainEqual({ op: "hide", id: view.id });
  expect(await next(() => setMode("visible"))).toEqual([{ op: "unhide", id: view.id }]);
  expect(await next(() => setMode("hidden"))).toEqual([{ op: "hide", id: view.id }]);
  expect(await next(() => setMode("hidden"))).toEqual([]);
});

test("a native component sends viewKind and its props as JSON, and re-sends them on change", async () => {
  const Color = defineNativeComponent<{ color: string }>({ viewKind: "app.color" });
  const [color, setColor] = createSignal("red");
  let id = 0;
  const ops = await mount(() => (
    <window>
      <Color ref={(r) => (id = r.id)} props={{ color: color() }} testID="c" style={{ hexpand: true }} />
    </window>
  ));
  expect(ops).toContainEqual({
    op: "create",
    id,
    widget: "NativeView",
    props: { viewKind: "app.color", props: '{"color":"red"}', testID: "c", style: { hexpand: true } },
  });
  expect(await next(() => setColor("blue"))).toEqual([{ op: "update", id, props: { props: '{"color":"blue"}' } }]);
});

test("a native component maps the native event name and sends commands through its ref", async () => {
  const View = defineNativeComponent<object, { n: number }, { to: string }>({ viewKind: "app.v" });
  const events: { name: string; data: { n: number } }[] = [];
  let native: NativeComponentRef<{ to: string }> | undefined;
  await mount(() => (
    <window>
      <View ref={native} props={{}} onNativeEvent={(e) => events.push(e)} />
    </window>
  ));
  registry.get(native!.id)!.handlers[eventForHandler("nativeview", "onNativeEvent")!]!({ nativeName: "pressed", data: { n: 2 } });
  expect(events).toEqual([{ name: "pressed", data: { n: 2 } }]);
  widgetCommands.length = 0;
  native!.send("reset", { to: "x" });
  expect(widgetCommands).toEqual([{ id: native!.id, command: "reset", arg: { to: "x" } }]);
});

test("a render error an <Errored> boundary catches is reported non-fatal and the tree keeps updating", async () => {
  setUnhandledErrorPolicy({ log: false });
  const reports: { message: string; kind: string; fatal: boolean }[] = [];
  const off = onUnhandledError((e, ctx) => reports.push({ message: e.message, kind: ctx.kind, fatal: ctx.fatal }));
  const [armed, setArmed] = createSignal(false);
  const [n, setN] = createSignal(0);
  const Boom = (): never => {
    throw new Error("render-throw");
  };
  let fallbackId = 0;
  await mount(() => (
    <window>
      <label>{n()}</label>
      <Errored fallback={(e) => <label ref={(l) => (fallbackId = l.id)} text={`caught: ${(e() as Error).message}`} />}>
        <Show when={armed()}>
          <Boom />
        </Show>
      </Errored>
    </window>
  ));
  const ops = await next(() => setArmed(true));
  expect(ops).toContainEqual(expect.objectContaining({ op: "create", id: fallbackId, props: { text: "caught: render-throw" } }));
  expect(reports).toEqual([{ message: "render-throw", kind: "renderCaught", fatal: false }]);
  expect((await next(() => setN(1))).map((o) => o.op)).toEqual(["setText"]);
  off();
});

test("a render error no boundary catches exits the process, whatever the policy says", async () => {
  const child = Bun.spawn(["bun", "--preload", "./src/register.ts", "./src/fixtures/uncaught-render.tsx"], {
    cwd: `${import.meta.dir}/..`,
    stdout: "pipe",
    stderr: "pipe",
  });
  const [code, out, err] = await Promise.all([child.exited, new Response(child.stdout).text(), new Response(child.stderr).text()]);
  expect(out).not.toContain("STILL_ALIVE");
  expect(err).toContain("[nd] render error: uncaught-render");
  expect(err).not.toContain("caught by boundary");
  expect(code).toBe(1);
});
