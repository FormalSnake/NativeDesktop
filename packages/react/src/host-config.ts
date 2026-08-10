import { DiscreteEventPriority, ContinuousEventPriority, DefaultEventPriority } from "react-reconciler/constants";
import { nextNodeId } from "./ids.ts";
import { Batch, NodeRegistry, type Handler } from "./ops.ts";
import { intrinsicToName, widgetEvents, handlerPropNames, widgetPlatforms } from "./generated/schema-meta.ts";
import type { WidgetType } from "./generated/intrinsics.ts";
import { validateStyle } from "./style-validate.ts";
import { validateCssClasses } from "./css-classes-validate.ts";
import { isHot } from "./hmr.ts";
import { Platform } from "./platform.ts";

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

function collectHandlers(type: string, props: Record<string, unknown>): Record<string, Handler> {
  const out: Record<string, Handler> = {};
  for (const ev of widgetEvents[type] ?? []) {
    const h = props[ev.handler];
    if (typeof h === "function") out[ev.name] = h as Handler;
  }
  return out;
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
  for (const h of handlerPropNames[inst.type] ?? []) delete props[h];
  activeBatch.push({ op: "create", id: inst.id, widget: intrinsicToName[inst.type] as "Window" | "Box" | "Label" | "Button", props });
  registry.register({ id: inst.id, type: inst.type, props: inst.props, handlers: collectHandlers(inst.type, inst.props) });
  for (const child of inst.children) {
    emitCreateIfNew(child);
    activeBatch.push({ op: "append", parent: inst.id, child: child.id });
  }
}

export function setPriorityFor(kind: "discrete" | "continuous" | "default"): void {
  currentUpdatePriority = kind === "discrete" ? DiscreteEventPriority : kind === "continuous" ? ContinuousEventPriority : DefaultEventPriority;
}

const PLATFORM_LABEL: Record<"macos" | "linux", string> = { macos: "macOS", linux: "Linux" };

// One-time-per-type dev warning when an intrinsic mounts on a platform its
// schema entry doesn't list (e.g. <trayitem> on linux) — it's not an error,
// the widget just renders as an invisible no-op there (see schema-meta's
// widgetPlatforms doc comment), but silently doing nothing is easy to miss.
// Gated on isHot() (ND_DEV=1, set only by `nd dev` — see hmr.ts) so a `nd
// build` release never pays for or prints this check.
const warnedPlatformMismatch = new Set<WidgetType>();
function checkPlatform(type: WidgetType): void {
  if (!isHot() || warnedPlatformMismatch.has(type)) return;
  const allowed = widgetPlatforms[type];
  if (!allowed || (allowed as readonly string[]).includes(Platform.os)) return;
  warnedPlatformMismatch.add(type);
  const label = allowed.map((p) => PLATFORM_LABEL[p]).join("/");
  console.warn(`<${type}> is ${label}-only; it renders nothing on ${Platform.os}. Gate it with Platform.os.`);
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
    const changed: Record<string, unknown> = {};
    for (const k of Object.keys(newProps)) {
      if (k === "children" || skip.includes(k)) continue;
      if (type === "label" && k === "text") continue; // routed through setText above
      if (newProps[k] !== oldProps[k]) changed[k] = newProps[k];
    }
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
