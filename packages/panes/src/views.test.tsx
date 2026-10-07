import { test, expect, beforeAll } from "bun:test";
import { flush } from "solid-js";
import { Batch, NodeRegistry, setSession, type Op } from "@nativedesktop/react/core";
import { render, nextCommit } from "@nativedesktop/react";
import { PaneTree, createPaneTree } from "./PaneTree.tsx";
import { seedPanes } from "./model.ts";

const commits: Op[][] = [];

beforeAll(() => {
  const batch = new Batch();
  setSession({
    ndp: {} as never,
    registry: new NodeRegistry(),
    batch,
    commit() {
      const ops = batch.drain();
      if (ops.length) commits.push(ops);
    },
    flush() {},
  });
});

const settle = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

test("createPaneTree writes nothing for an op that changes nothing, and latest() sees a staged write", () => {
  const panes = createPaneTree(seedPanes([{ n: 1 }, { n: 2 }]));
  const before = panes.model();
  panes.focus(before.focusedId);
  panes.setRatio("missing", 0.3);
  flush();
  expect(panes.model()).toBe(before);
  panes.setRatio("s3", 0.3);
  expect(panes.latest()).not.toBe(before);
  flush();
  expect(panes.model()).toBe(panes.latest());
});

test("a model op updates the live paned and leaves in place instead of recreating them", async () => {
  const panes = createPaneTree(seedPanes([{ n: 1 }, { n: 2 }]));
  globalThis.__nd_solid_mounted?.dispose();
  globalThis.__nd_solid_mounted = undefined;
  await settle();
  commits.length = 0;
  const done = nextCommit();
  await render(() => (
    <window>
      <PaneTree
        model={panes.model()}
        onChange={panes.setModel}
        renderLeaf={(leaf) => <label text={`${leaf.id}${leaf.focused ? "*" : ""}`} />}
      />
    </window>
  ));
  await done;
  const mounted = commits.flat();
  const paned = mounted.find((o) => o.op === "create" && o.widget === "Paned") as { id: number };
  expect(mounted.filter((o) => o.op === "create" && o.widget === "Label")).toHaveLength(2);

  commits.length = 0;
  panes.setRatio("s3", 0.3);
  panes.focus("2");
  await settle();
  const ops = commits.flat();
  expect(ops.some((o) => o.op === "create" || o.op === "remove")).toBe(false);
  expect(ops).toContainEqual({ op: "update", id: paned.id, props: { position: 0.3 } });
  expect(ops.filter((o) => o.op === "setText").map((o) => (o as { text: string }).text).sort()).toEqual(["1", "2*"]);
});
