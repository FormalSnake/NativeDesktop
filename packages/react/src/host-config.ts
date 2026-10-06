import { DiscreteEventPriority, ContinuousEventPriority, DefaultEventPriority } from "react-reconciler/constants";
import {
  Batch,
  NodeRegistry,
  nextNodeId,
  intrinsicToName,
  handlerPropNames,
  widgetRefProps,
  validateStyle,
  validateCssClasses,
  collectHandlers,
  refTargetId,
  propsEqual,
  removalValue,
  checkPlatform,
  type WidgetType,
} from "@nativedesktop/core";

export type { WidgetType };

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
  if ("style" in props) validateStyle(props.style);
  if ("cssClasses" in props) validateCssClasses(props.cssClasses);
  const text = textOf(props.children);
  if (inst.type === "label" && text !== undefined) props.text = text;
  delete props.children;
  resolveRefProps(inst, props);
  for (const h of handlerPropNames[inst.type] ?? []) delete props[h];
  activeBatch.push({ op: "create", id: inst.id, widget: intrinsicToName[inst.type] as "Window" | "Box" | "Label" | "Button", props });
  registry.register({ id: inst.id, type: inst.type, props: inst.props, handlers: collectHandlers(inst.type, inst.props) });
  for (const child of inst.children) {
    emitCreateIfNew(child);
    activeBatch.push({ op: "append", parent: inst.id, child: child.id });
  }
}

/// The wire id last sent for each of an instance's ref props. A React ref is
/// one object mutated in place, so oldProps and newProps hold the SAME ref and
/// a diff over resolved values reports no change on the very render where the
/// ref starts pointing at a node.
const lastRefIds = new WeakMap<Instance, Record<string, number>>();

/// Create path: swaps a ref-valued JSX prop (`anchorRef={buttonRef}`) for the
/// wire prop carrying the target's node id (`anchor: 7`), in the shallow copy
/// the caller is about to send.
///
/// React attaches host refs AFTER the mutation phase that emits a mount's
/// create op, so a ref to a node mounting in the same commit still reads null
/// here and the id lands on the next render that updates this widget. Both
/// backends resolve the id lazily for that reason.
function resolveRefProps(inst: Instance, props: Record<string, unknown>): void {
  const refs = widgetRefProps[inst.type];
  if (!refs) return;
  const sent: Record<string, number> = {};
  for (const [refName, wireName] of Object.entries(refs)) {
    if (!(refName in props)) continue;
    const id = refTargetId(props[refName]);
    delete props[refName];
    if (id === undefined) continue;
    props[wireName] = id;
    sent[wireName] = id;
  }
  lastRefIds.set(inst, sent);
}

/// Update path: diffs each ref prop against the id last sent for it, since the
/// ref object itself says nothing about whether the node behind it changed.
function diffRefProps(inst: Instance, props: Record<string, unknown>, changed: Record<string, unknown>): void {
  const refs = widgetRefProps[inst.type];
  if (!refs) return;
  let sent = lastRefIds.get(inst);
  if (!sent) {
    sent = {};
    lastRefIds.set(inst, sent);
  }
  for (const [refName, wireName] of Object.entries(refs)) {
    if (wireName in props) continue; // an explicit id wins, and the generic diff owns it
    const id = refName in props ? refTargetId(props[refName]) : undefined;
    if (id === sent[wireName]) continue;
    if (id === undefined) {
      changed[wireName] = null;
      delete sent[wireName];
      continue;
    }
    changed[wireName] = id;
    sent[wireName] = id;
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
  // Required once createPortal is used (renderer.ts's moveNode mechanism): the
  // portal's off-window pool container needs no pre-mount preparation here —
  // its children are created as detached widgets and only attached to a window
  // later by the host-level reparent (Tree.reparent).
  preparePortalMount: () => {},

  // ---- render phase: PURE, no socket, no host widgets ----
  createInstance(type: WidgetType, props: Record<string, unknown>): Instance {
    checkPlatform(type);
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
    if ("style" in newProps) validateStyle(newProps.style);
    if ("cssClasses" in newProps) validateCssClasses(newProps.cssClasses);
    // React 19: no prepareUpdate — diff here.
    if (type === "label") {
      const t = textOf(newProps.children) ?? (newProps.text as string | undefined);
      const old = textOf(oldProps.children) ?? (oldProps.text as string | undefined);
      if (t !== undefined && t !== old) activeBatch.push({ op: "setText", id: inst.id, text: t });
    }
    const skip = handlerPropNames[type] ?? [];
    const refs = widgetRefProps[type];
    // A ref prop never crosses the wire under its JSX name; diffRefProps below
    // owns the wire prop it resolves to.
    const wire = (k: string) => k !== "children" && !skip.includes(k)
      && !(type === "label" && k === "text") && !(refs !== undefined && k in refs);
    const changed: Record<string, unknown> = {};
    for (const k of Object.keys(newProps)) {
      if (!wire(k)) continue; // "text" on a label is routed through setText above
      // `label={cond ? x : undefined}` keeps the key with no value, which JSON
      // drops from the op: it is a removal like a key the render left out.
      if (newProps[k] === undefined) {
        if (oldProps[k] !== undefined) changed[k] = removalValue(k);
        continue;
      }
      if (!propsEqual(newProps[k], oldProps[k])) changed[k] = newProps[k];
    }
    // A prop the new render dropped has to reach the host too, or the widget
    // keeps the last value it was given. NDP has no removal tag (an `update`
    // op carries a props object, nothing else), so null is the removal marker:
    // it is the one value that cannot be a legitimate typed prop, and both
    // encodings carry it (JSON null, binary value tag 0x00). The generated
    // appliers turn it back into the prop's schema default
    // (ndApplyDroppedDefaults, tools/codegen.ts).
    for (const k of Object.keys(oldProps)) {
      if (k in newProps || !wire(k)) continue;
      changed[k] = removalValue(k);
    }
    diffRefProps(inst, newProps, changed);
    if (Object.keys(changed).length) activeBatch.push({ op: "update", id: inst.id, props: changed });
    inst.props = newProps;
    // Re-register handlers so events route to the latest closures.
    const rec = registry.get(inst.id);
    if (rec) rec.handlers = collectHandlers(type, newProps);
  },
  commitTextUpdate() {}, // labels handle their own text via commitUpdate

  hideInstance(inst: Instance) { activeBatch.push({ op: "hide", id: inst.id }); },
  unhideInstance(inst: Instance) { activeBatch.push({ op: "unhide", id: inst.id }); },
  hideTextInstance() {},
  unhideTextInstance() {},

  getPublicInstance: (i: Instance) => i,
  // removeChild only sees the root of a removed subtree; React calls this for
  // every host instance under it as well.
  detachDeletedInstance(inst: Instance) {
    registry.unregister(inst.id);
  },
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
