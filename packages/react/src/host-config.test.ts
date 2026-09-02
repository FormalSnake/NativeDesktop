import { test, expect, spyOn } from "bun:test";
import { hostConfig, bindCommitTargets, type Container, type Instance, type WidgetType } from "./host-config.ts";
import { Batch, NodeRegistry, type Handler } from "./ops.ts";

/// Mounts one intrinsic through the commit path and answers the handler map
/// the event router would dispatch against.
function handlersOf(type: WidgetType, props: Record<string, unknown>): Record<string, Handler> {
  const registry = new NodeRegistry();
  bindCommitTargets(new Batch(), registry);
  const inst = hostConfig.createInstance(type, props) as Instance;
  hostConfig.appendChildToContainer({ rootId: null } as Container, inst);
  return registry.get(inst.id)!.handlers;
}

/// Mounts one intrinsic, throws the create ops away, then answers the ops a
/// re-render with `newProps` emits.
function updateOps(type: WidgetType, oldProps: Record<string, unknown>, newProps: Record<string, unknown>) {
  const batch = new Batch();
  bindCommitTargets(batch, new NodeRegistry());
  const inst = hostConfig.createInstance(type, oldProps) as Instance;
  hostConfig.appendChildToContainer({ rootId: null } as Container, inst);
  batch.drain();
  hostConfig.commitUpdate(inst, type, oldProps, newProps);
  return batch.drain();
}

// `style={{ padding: 12 }}` is a fresh object on every render, so an identity
// diff calls it changed every time and ships the whole prop to the host for a
// render that touched nothing.
test("an object prop with unchanged contents emits no update op", () => {
  expect(updateOps("box", { style: { padding: 12, hexpand: true } }, { style: { padding: 12, hexpand: true } })).toEqual([]);
  expect(updateOps("box", { style: { font: { fontSize: 13 } } }, { style: { font: { fontSize: 13 } } })).toEqual([]);
  expect(updateOps("box", { cssClasses: ["card"] }, { cssClasses: ["card"] })).toEqual([]);
});

test("an object prop whose contents changed still emits the update", () => {
  expect(updateOps("box", { style: { padding: 12 } }, { style: { padding: 16 } })).toEqual([
    { op: "update", id: expect.any(Number), props: { style: { padding: 16 } } },
  ]);
  expect(updateOps("box", { style: { font: { fontSize: 13 } } }, { style: { font: { fontSize: 15 } } })).toEqual([
    { op: "update", id: expect.any(Number), props: { style: { font: { fontSize: 15 } } } },
  ]);
  expect(updateOps("box", { cssClasses: ["card"] }, { cssClasses: ["card", "flat"] })).toEqual([
    { op: "update", id: expect.any(Number), props: { cssClasses: ["card", "flat"] } },
  ]);
});

// A prop the new render dropped used to vanish from the diff entirely, leaving
// the widget on its last value forever. NDP has no removal tag, so null is it.
test("a prop the new render dropped reaches the host as null", () => {
  expect(updateOps("box", { spacing: 8, orientation: "vertical" }, { orientation: "vertical" })).toEqual([
    { op: "update", id: expect.any(Number), props: { spacing: null } },
  ]);
});

test("dropped children and handler props are never sent as removals", () => {
  expect(updateOps("button", { label: "Go", onClick: () => {}, children: "Go" }, { label: "Go" })).toEqual([]);
});

test("a declared handler prop registers under the schema's event name", () => {
  expect(Object.keys(handlersOf("button", { onClick: () => {} }))).toEqual(["clicked"]);
  expect(Object.keys(handlersOf("checkbox", { onToggled: () => {} }))).toEqual(["toggled"]);
});

// The Button schema declares `clicked` under the prop name `onClick`, so a
// handler written as `onClicked` registers nothing, while the click RPC still
// answers dispatched:true and the native `clicked` signal is still connected,
// which makes the widget look dead rather than misspelled. The warning is the
// only signal an app gets, so it is pinned here.
test("an undeclared on* prop registers nothing and warns once, naming the declared events", () => {
  const warn = spyOn(console, "warn").mockImplementation(() => {});
  try {
    expect(Object.keys(handlersOf("button", { onClick: () => {}, onClicked: () => {} }))).toEqual(["clicked"]);
    const text = warn.mock.calls.map((c) => c.join(" ")).join("\n");
    expect(text).toContain('<button> has no "onClicked" event');
    expect(text).toContain("Declared events: onClick, onHoverChanged, onDragStarted, onDragEnded, onDragOver, onDropped.");
  } finally {
    warn.mockRestore();
  }
});

test("a non-function on* prop is left alone", () => {
  const warn = spyOn(console, "warn").mockImplementation(() => {});
  try {
    handlersOf("button", { onceOnly: "x", onClicks: 3 });
    expect(warn.mock.calls).toHaveLength(0);
  } finally {
    warn.mockRestore();
  }
});
