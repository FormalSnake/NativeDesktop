import { DiscreteEventPriority, ContinuousEventPriority, DefaultEventPriority } from "react-reconciler/constants";
import { nextNodeId } from "./ids.ts";
import { Batch, NodeRegistry } from "./ops.ts";

export type WidgetType = "window" | "box" | "label" | "button";
const WIDGET: Record<WidgetType, "Window" | "Box" | "Label" | "Button"> = {
  window: "Window", box: "Box", label: "Label", button: "Button",
};

export interface Instance {
  id: number;
  type: WidgetType;
  props: Record<string, unknown>;
  /** Populated by appendInitialChild during render; only meaningful until the
   *  instance's first commit-time attach, at which point emitCreateIfNew
   *  flushes it into create+append ops (see note below). */
  children: Instance[];
}

// Set by the renderer immediately before updateContainer / on each commit.
export let activeBatch: Batch = new Batch();
export let registry: NodeRegistry = new NodeRegistry();
export function bindCommitTargets(b: Batch, r: NodeRegistry): void { activeBatch = b; registry = r; }

let currentUpdatePriority = DefaultEventPriority;

function textOf(children: unknown): string | undefined {
  if (typeof children === "string") return children;
  if (typeof children === "number") return String(children);
  if (Array.isArray(children) && children.every((c) => typeof c === "string" || typeof c === "number"))
    return children.join("");
  return undefined;
}

export interface Container { rootId: number | null }

// React only calls appendChild/appendChildToContainer for the TOP of a
// freshly-built subtree — a brand-new parent's own children were already
// linked in memory via appendInitialChild during render, with no further
// commit-phase callback per descendant. So a first-time attach must walk
// inst.children recursively, emitting `create` + `append` for every
// not-yet-registered descendant, not just `inst` itself.
function emitCreateIfNew(inst: Instance): void {
  if (registry.get(inst.id)) return;
  const props: Record<string, unknown> = { ...inst.props };
  const text = textOf(props.children);
  if (inst.type === "label" && text !== undefined) props.text = text;
  delete props.children;
  delete props.onClick;
  activeBatch.push({ op: "create", id: inst.id, widget: WIDGET[inst.type], props });
  registry.register({ id: inst.id, type: inst.type, props: inst.props, onClick: inst.props.onClick as (() => void) | undefined });
  for (const child of inst.children) {
    emitCreateIfNew(child);
    activeBatch.push({ op: "append", parent: inst.id, child: child.id });
  }
}

export function setPriorityFor(kind: "discrete" | "continuous" | "default"): void {
  currentUpdatePriority = kind === "discrete" ? DiscreteEventPriority : kind === "continuous" ? ContinuousEventPriority : DefaultEventPriority;
}

export const hostConfig = {
  supportsMutation: true,
  supportsPersistence: false,
  supportsHydration: false,
  isPrimaryRenderer: true,
  noTimeout: -1 as const,
  supportsMicrotasks: true,
  scheduleMicrotask: (fn: () => void) => queueMicrotask(fn),
  scheduleTimeout: (fn: (...args: unknown[]) => void, delay?: number) => setTimeout(fn, delay),
  cancelTimeout: (id: ReturnType<typeof setTimeout>) => clearTimeout(id),

  getRootHostContext: () => ({ root: true }), // non-null sentinel
  getChildHostContext: (parent: unknown) => parent, // non-null (parent is non-null)
  prepareForCommit: () => null,
  resetAfterCommit: () => {}, // renderer overrides via wrapper; see renderer.ts
  clearContainer: () => {},

  // ---- render phase: PURE, no socket, no host widgets ----
  createInstance(type: WidgetType, props: Record<string, unknown>): Instance {
    const id = nextNodeId();
    return { id, type, props, children: [] };
  },
  createTextInstance(text: string): { text: string } {
    return { text }; // folded into the parent label's text; see shouldSetTextContent
  },
  shouldSetTextContent: (type: WidgetType) => type === "label",
  // Pure: no ops emitted, but the parent-child link must be recorded here —
  // for a freshly-built subtree this is the ONLY callback informing us of
  // structure; emitCreateIfNew replays it into ops at first commit-time attach.
  appendInitialChild(parent: Instance, child: Instance) { parent.children.push(child); },
  finalizeInitialChildren: () => false,

  // ---- commit phase: emit ops into activeBatch ----
  appendChild(parent: Instance, child: Instance) {
    emitCreateIfNew(child);
    activeBatch.push({ op: "append", parent: parent.id, child: child.id });
  },
  appendChildToContainer(_container: Container, child: Instance) {
    emitCreateIfNew(child); // the window instance itself: no parent op needed
  },
  insertBefore(parent: Instance, child: Instance, before: Instance) {
    emitCreateIfNew(child);
    activeBatch.push({ op: "insertBefore", parent: parent.id, child: child.id, before: before.id });
  },
  insertInContainerBefore(_c: Container, child: Instance, _b: Instance) {
    emitCreateIfNew(child);
  },
  removeChild(_parent: Instance, child: Instance) {
    activeBatch.push({ op: "remove", id: child.id });
    registry.unregister(child.id);
  },
  removeChildFromContainer(_c: Container, child: Instance) {
    activeBatch.push({ op: "remove", id: child.id });
    registry.unregister(child.id);
  },

  commitUpdate(inst: Instance, type: WidgetType, oldProps: Record<string, unknown>, newProps: Record<string, unknown>) {
    // React 19: no prepareUpdate — diff here.
    if (type === "label") {
      const t = textOf(newProps.children) ?? (newProps.text as string | undefined);
      const old = textOf(oldProps.children) ?? (oldProps.text as string | undefined);
      if (t !== undefined && t !== old) activeBatch.push({ op: "setText", id: inst.id, text: t });
    }
    const changed: Record<string, unknown> = {};
    for (const k of Object.keys(newProps)) {
      if (k === "children" || k === "onClick") continue;
      if (type === "label" && k === "text") continue; // routed through setText above
      if (newProps[k] !== oldProps[k]) changed[k] = newProps[k];
    }
    if (Object.keys(changed).length) activeBatch.push({ op: "update", id: inst.id, props: changed });
    inst.props = newProps;
    // Re-register handler so events route to the latest closure.
    const rec = registry.get(inst.id);
    if (rec) rec.onClick = newProps.onClick as (() => void) | undefined;
  },
  commitTextUpdate() {}, // labels handle their own text via commitUpdate

  hideInstance(inst: Instance) { activeBatch.push({ op: "hide", id: inst.id }); },
  unhideInstance(inst: Instance) { activeBatch.push({ op: "unhide", id: inst.id }); },
  hideTextInstance() {},
  unhideTextInstance() {},

  getPublicInstance: (i: Instance) => i,
  detachDeletedInstance() {},
  maySuspendCommit: () => false,
  preloadInstance: () => true,
  startSuspendingCommit() {},
  suspendInstance() {},
  waitForCommitToBeReady: () => null,

  resolveUpdatePriority: () => currentUpdatePriority,
  getCurrentUpdatePriority: () => currentUpdatePriority,
  setCurrentUpdatePriority: (p: number) => { currentUpdatePriority = p; },
  shouldAttemptEagerTransition: () => false,
  trackSchedulerEvent() {},
  resolveEventType: () => null,
  resolveEventTimeStamp: () => -1.1,
  requestPostPaintCallback() {},
};
