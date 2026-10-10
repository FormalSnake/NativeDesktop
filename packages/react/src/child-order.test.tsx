// Adjacent expressions share one marker in the universal compiler's output,
// so an emptied one used to refill after its later neighbours. Replays the
// ops into host-style child lists and toggles the footer of a browser sidebar
// in a seeded random order.
import { test, expect, beforeAll } from "bun:test";
import { createSignal, For, Show } from "solid-js";
import { Batch, NodeRegistry, setSession, type Op } from "@nativedesktop/react/core";
import { render, nextCommit } from "./renderer.ts";

const commits: Op[][] = [];
beforeAll(() => {
  const batch = new Batch();
  setSession({
    ndp: { sendWidgetCommand() {}, sendRuntimeError() {} } as never,
    registry: new NodeRegistry(),
    batch,
    commit() { const ops = batch.drain(); if (ops.length) commits.push(ops); },
    flush() {},
  });
});
const settle = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

// A host's child lists, replayed from the ops.
const kids = new Map<number, number[]>();
const tid = new Map<number, string>();
const parentOf = new Map<number, number>();
function replay(ops: Op[]) {
  for (const o of ops as any[]) {
    if (o.op === "create") { kids.set(o.id, []); if (o.props?.testID) tid.set(o.id, o.props.testID); }
    else if (o.op === "update" && o.props?.testID) tid.set(o.id, o.props.testID);
    else if (o.op === "append" || o.op === "insertBefore") {
      const old = parentOf.get(o.child);
      if (old !== undefined) kids.set(old, kids.get(old)!.filter((c) => c !== o.child));
      const list = kids.get(o.parent) ?? [];
      if (o.op === "append") list.push(o.child);
      else { const i = list.indexOf(o.before); if (i < 0) throw new Error("before not a child"); list.splice(i, 0, o.child); }
      kids.set(o.parent, list); parentOf.set(o.child, o.parent);
    } else if (o.op === "remove") {
      const p = parentOf.get(o.id); if (p !== undefined) kids.set(p, kids.get(p)!.filter((c) => c !== o.id)); parentOf.delete(o.id);
    }
  }
}
const order = () => { const bar = [...tid].find(([, t]) => t === "bar")![0]; return kids.get(bar)!.map((c) => tid.get(c)); };

test("conditional footer items land at their JSX position in any toggle order", async () => {
  const [site, setSite] = createSignal(false);
  const [zoom, setZoom] = createSignal(false);
  const [pop, setPop] = createSignal(false);
  const [ext, setExt] = createSignal(false);
  const [pins, setPins] = createSignal<string[]>([]);
  const [dl, setDl] = createSignal(false);
  const [menu, setMenu] = createSignal(false);
  const [gtk] = createSignal(false);
  function Foot(props: any) {
    return (
      <box testID="bar" orientation="horizontal">
        <button testID="settings" />
        {props.siteInfo}
        {props.zoom}
        {props.extensions}
        {props.downloads}
        {props.windowMenu}
        <box testID="spacer" orientation="horizontal" />
        <Show when={gtk()}><button testID="plus" /></Show>
      </box>
    );
  }
  commits.length = 0;
  const done = nextCommit();
  await render((() => (
    <window title="t">
      <Foot
        siteInfo={<Show when={site()}><box testID="site" orientation="horizontal" /></Show>}
        zoom={<><Show when={pop()}><button testID="popups" /></Show><Show when={zoom()}><box testID="zoom" orientation="horizontal" /></Show></>}
        extensions={<Show when={ext()}><For each={pins()}>{(p) => <box testID={`pin-${p}`} orientation="horizontal" />}</For><button testID="puzzle" /></Show>}
        downloads={<Show when={dl()}><box testID="downloads" orientation="horizontal" /></Show>}
        windowMenu={<Show when={menu()}><menubutton testID="menu" /></Show>}
      />
    </window>
  )) as never);
  await done;
  replay(commits.flat());
  const canon = ["settings", "site", "popups", "zoom", "pin-a", "pin-b", "puzzle", "downloads", "menu", "spacer"];
  const setters: Record<string, (v: boolean) => void> = {
    site: setSite, zoom: setZoom, popups: setPop, ext: setExt, downloads: setDl, menu: setMenu,
    pins: (v) => setPins(v ? ["a", "b"] : []),
  };
  const state: Record<string, boolean> = {};
  let seed = 7;
  const rnd = () => (seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff;
  const names = Object.keys(setters);
  for (let i = 0; i < 400; i++) {
    const n = names[Math.floor(rnd() * names.length)]!;
    state[n] = !state[n];
    commits.length = 0;
    setters[n]!(state[n]!);
    await settle();
    replay(commits.flat());
    const want = canon.filter((t) => {
      if (t === "settings" || t === "spacer") return true;
      if (t === "puzzle") return state.ext;
      if (t.startsWith("pin-")) return state.ext && state.pins;
      return state[t];
    });
    expect({ step: i, toggled: n, got: order() }).toEqual({ step: i, toggled: n, got: want });
  }
});
