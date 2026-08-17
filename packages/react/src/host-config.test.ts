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
    expect(text).toContain("Declared events: onClick, onHoverChanged.");
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
