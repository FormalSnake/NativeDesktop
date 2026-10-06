import { createRenderer } from "@solidjs/universal";
import { Errored, children, configureClientErrors, createRenderEffect, flush, onCleanup, type Element as SolidElement } from "solid-js";
import {
  connect,
  installErrorHandlers,
  reportRenderError,
  nextNodeId,
  intrinsicToName,
  handlerPropNames,
  widgetRefProps,
  validateStyle,
  validateCssClasses,
  eventForHandler,
  warnUnknownHandler,
  refTargetId,
  propsEqual,
  removalValue,
  checkPlatform,
  type Handler,
  type Session,
  type WidgetType,
} from "@nativedesktop/core";
import { lat, type Op } from "@nativedesktop/core";

type Kind = WidgetType | "#text" | "#root";

/// The renderer's own retained tree. Solid builds and moves nodes DOM-style
/// (detached subtrees first, then one insert into a live parent), so the tree
/// is mirrored here and turned into ops only when a node reaches a root.
export interface SolidNode {
  id: number;
  type: Kind;
  /** JSX-named props as last set, handlers excluded. */
  props: Record<string, unknown>;
  /** Wire event name -> handler; this object is what the registry dispatches on. */
  handlers: Record<string, Handler>;
  text: string;
  parent: SolidNode | null;
  firstChild: SolidNode | null;
  lastChild: SolidNode | null;
  prev: SolidNode | null;
  next: SolidNode | null;
  /** A create op for the current id is on the wire or in the open batch. */
  created: boolean;
  /** The removed widget's id is dead host-side; a re-insert creates a fresh one. */
  needsId: boolean;
  /** Props of this node's create op while it sits in the open batch. */
  pendingCreate: Record<string, unknown> | null;
  /** Props of this node's update op while it sits in the open batch. */
  pendingUpdate: Record<string, unknown> | null;
  /** Last text sent for a label. */
  sentText: string | undefined;
  /** Hidden by an <Activity>; a fresh create of this node comes up hidden too. */
  hidden: boolean;
}

function makeNode(type: Kind): SolidNode {
  return {
    id: type === "#text" || type === "#root" ? 0 : nextNodeId(),
    type,
    props: {},
    handlers: {},
    text: "",
    parent: null,
    firstChild: null,
    lastChild: null,
    prev: null,
    next: null,
    created: false,
    needsId: false,
    pendingCreate: null,
    pendingUpdate: null,
    sentText: undefined,
    hidden: false,
  };
}

let session: Session | undefined;
let scheduled = false;
const touched: SolidNode[] = [];
const commitWaiters: (() => void)[] = [];

function emit(op: Op): void {
  session!.batch.push(op);
  if (!scheduled) {
    scheduled = true;
    queueMicrotask(commitNow);
  }
}

function commitNow(): void {
  lat("js.commitStart");
  // Drain Solid's own queue first so effects a signal write staged this tick
  // land in this CommitBatch rather than the next.
  flush();
  scheduled = false;
  shipBatch();
  for (const w of commitWaiters.splice(0)) w();
}

/// Sends the ops emitted so far, without draining Solid's queue: this also
/// runs from inside Solid's flush (a command sent from a ref or onSettled).
function shipBatch(): void {
  for (const n of touched) {
    n.pendingCreate = null;
    n.pendingUpdate = null;
  }
  touched.length = 0;
  updatesLast(session!.batch);
  session!.commit();
}

/// React's commit applies a node's prop update after the inserts and removals
/// of the same commit, and hosts depend on it: an AppKit popover opened
/// before its new content is attached sizes for the old content and then
/// dismisses itself. Solid runs a prop effect wherever it falls, so updates
/// move behind the batch's structural ops here. An update to a node the batch
/// also removes stays ahead of the remove, since its id is dead after it.
function updatesLast(batch: Session["batch"]): void {
  const ops = batch.drain();
  const removed = new Set<number>();
  for (const op of ops) if (op.op === "remove") removed.add(op.id);
  const late: Op[] = [];
  for (const op of ops) {
    if (op.op === "update" && !removed.has(op.id)) late.push(op);
    else batch.push(op);
  }
  for (const op of late) batch.push(op);
}

/// Resolves once the next CommitBatch has been handed to NDP.
export function nextCommit(): Promise<void> {
  return new Promise((resolve) => commitWaiters.push(resolve));
}

function isLive(node: SolidNode): boolean {
  let n: SolidNode | null = node;
  while (n) {
    if (n.type === "#root") return true;
    n = n.parent;
  }
  return false;
}

function labelText(label: SolidNode): string | undefined {
  let text: string | undefined;
  for (let c = label.firstChild; c; c = c.next) if (c.type === "#text") text = (text ?? "") + c.text;
  if (text !== undefined) return text;
  const t = label.props.text;
  return typeof t === "string" ? t : t === undefined || t === null ? undefined : String(t);
}

function syncLabelText(label: SolidNode): void {
  if (label.type !== "label" || !label.created) return;
  const t = labelText(label);
  if (label.pendingCreate) {
    if (t === undefined) delete label.pendingCreate.text;
    else label.pendingCreate.text = t;
    label.sentText = t;
    return;
  }
  if (t === undefined || t === label.sentText) return;
  label.sentText = t;
  emit({ op: "setText", id: label.id, text: t });
}

function wireValue(node: SolidNode, name: string, value: unknown): [string, unknown] | null {
  const ref = widgetRefProps[node.type]?.[name];
  if (ref !== undefined) return [ref, refTargetId(value) ?? null];
  if (typeof value === "function") return null;
  return [name, value];
}

function createProps(node: SolidNode): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const name of Object.keys(node.props)) {
    if (node.type === "label" && name === "text") continue;
    const value = node.props[name];
    if (value === undefined) continue;
    const wire = wireValue(node, name, value);
    if (!wire || wire[1] === null) continue;
    out[wire[0]] = wire[1];
  }
  if ("style" in out) validateStyle(out.style);
  if ("cssClasses" in out) validateCssClasses(out.cssClasses);
  if (node.type === "label") {
    const t = labelText(node);
    if (t !== undefined) out.text = t;
    node.sentText = t;
  }
  return out;
}

function mount(node: SolidNode): void {
  if (node.type === "#text" || node.created) return;
  if (node.needsId) {
    node.id = nextNodeId();
    node.needsId = false;
  }
  const props = createProps(node);
  emit({ op: "create", id: node.id, widget: intrinsicToName[node.type] as "Window", props });
  if (node.hidden) emit({ op: "hide", id: node.id });
  node.created = true;
  node.pendingCreate = props;
  touched.push(node);
  session!.registry.register(node);
  for (let c = node.firstChild; c; c = c.next) {
    if (c.type === "#text") continue;
    mount(c);
    emit({ op: "append", parent: node.id, child: c.id });
  }
}

function unmount(node: SolidNode): void {
  if (node.type === "#text" || !node.created) return;
  node.created = false;
  node.needsId = true;
  node.pendingCreate = null;
  node.pendingUpdate = null;
  session!.registry.unregister(node.id);
  for (let c = node.firstChild; c; c = c.next) unmount(c);
}

function unlink(node: SolidNode): void {
  const p = node.parent!;
  if (node.prev) node.prev.next = node.next;
  else p.firstChild = node.next;
  if (node.next) node.next.prev = node.prev;
  else p.lastChild = node.prev;
  node.parent = node.prev = node.next = null;
}

function link(parent: SolidNode, node: SolidNode, anchor: SolidNode | null): void {
  node.parent = parent;
  if (anchor) {
    node.next = anchor;
    node.prev = anchor.prev;
    if (anchor.prev) anchor.prev.next = node;
    else parent.firstChild = node;
    anchor.prev = node;
  } else {
    node.prev = parent.lastChild;
    node.next = null;
    if (parent.lastChild) parent.lastChild.next = node;
    else parent.firstChild = node;
    parent.lastChild = node;
  }
}

function nextWidgetSibling(node: SolidNode): SolidNode | null {
  for (let s = node.next; s; s = s.next) if (s.type !== "#text") return s;
  return null;
}

function removeFromParent(node: SolidNode): void {
  const parent = node.parent!;
  unlink(node);
  if (node.type === "#text") {
    syncLabelText(parent);
    return;
  }
  if (node.created) {
    emit({ op: "remove", id: node.id });
    unmount(node);
  }
}

function insertNode(parent: SolidNode, node: SolidNode, anchor?: SolidNode): void {
  // DOM semantics: inserting a node that is already in the tree moves it. A
  // move within one live parent keeps the widget; a move to another parent
  // is a remove plus a fresh create.
  const moving = node.parent === parent && node.created && node.type !== "#text";
  if (node.parent) {
    if (moving) unlink(node);
    else removeFromParent(node);
  }
  link(parent, node, anchor ?? null);
  if (node.type === "#text") {
    syncLabelText(parent);
    return;
  }
  if (!isLive(parent)) return;
  mount(node);
  if (parent.type === "#root") return;
  const before = nextWidgetSibling(node);
  if (before && before.created) emit({ op: "insertBefore", parent: parent.id, child: node.id, before: before.id });
  else emit({ op: "append", parent: parent.id, child: node.id });
}

function setProperty(node: SolidNode, name: string, value: unknown, prev?: unknown): void {
  if (node.type === "#text" || node.type === "#root") return;
  if ((handlerPropNames[node.type] ?? []).includes(name)) {
    const event = eventForHandler(node.type, name)!;
    if (typeof value === "function") node.handlers[event] = value as Handler;
    else delete node.handlers[event];
    return;
  }
  warnUnknownHandler(node.type, name, value);
  if (value === undefined) delete node.props[name];
  else node.props[name] = value;
  if (node.type === "label" && name === "text") {
    syncLabelText(node);
    return;
  }
  if (!node.created) return;
  const wire = wireValue(node, name, value);
  if (!wire) return;
  const [key, v] = wire;
  if (name === "style" && v !== undefined) validateStyle(v);
  if (name === "cssClasses" && v !== undefined) validateCssClasses(v);
  if (node.pendingCreate) {
    if (v === undefined || v === null) delete node.pendingCreate[key];
    else node.pendingCreate[key] = v;
    return;
  }
  let out: unknown;
  if (key !== name) {
    if (v === (refTargetId(prev) ?? null)) return;
    out = v;
  } else if (v === undefined) {
    if (prev === undefined) return;
    out = removalValue(key);
  } else {
    if (propsEqual(v, prev)) return;
    out = v;
  }
  if (node.pendingUpdate) {
    node.pendingUpdate[key] = out;
    return;
  }
  const props: Record<string, unknown> = { [key]: out };
  node.pendingUpdate = props;
  touched.push(node);
  emit({ op: "update", id: node.id, props });
}

const renderer = createRenderer<SolidNode>({
  createElement(tag: string, staticProps?: Record<string, unknown>): SolidNode {
    checkPlatform(tag as WidgetType);
    const node = makeNode(tag as WidgetType);
    if (staticProps) for (const name of Object.keys(staticProps)) setProperty(node, name, staticProps[name]);
    return node;
  },
  createTextNode(value: string): SolidNode {
    const node = makeNode("#text");
    node.text = String(value);
    return node;
  },
  replaceText(textNode: SolidNode, value: string): void {
    textNode.text = String(value);
    if (textNode.parent) syncLabelText(textNode.parent);
  },
  isTextNode: (node) => node.type === "#text",
  setProperty,
  insertNode,
  removeNode(_parent: SolidNode, node: SolidNode): void {
    if (node.parent) removeFromParent(node);
  },
  getParentNode: (node) => node.parent ?? undefined,
  getFirstChild: (node) => node.firstChild ?? undefined,
  getNextSibling: (node) => node.next ?? undefined,
});

export const {
  effect,
  memo,
  createComponent,
  createElement,
  createTextNode,
  insert,
  spread,
  setProp,
  mergeProps,
  applyRef,
  ref,
} = renderer;
export { insertNode };

/// An off-window container whose children become detached native widgets:
/// created and kept alive, but shown in no window until `moveNode` attaches
/// them somewhere. Create pools once (module scope), never inside a component.
export interface Pool {
  readonly node: SolidNode;
}

export function createPool(): Pool {
  return { node: makeNode("#root") };
}

const defaultPool: Pool = createPool();

/// Renders `children` into `pool` (default: one shared process-lifetime pool)
/// while their owner stays at THIS position in the component tree, so moving
/// a tab between windows never disposes them. Pair with `moveNode` to
/// relocate the live native widget; a <webview> keeps its page that way.
export function Portal(props: { pool?: Pool; children?: SolidElement }): SolidElement {
  const pool = (props.pool ?? defaultPool).node;
  // Each portal owns a span of the pool between its own marker and the next,
  // so two portals into one pool never replace each other's nodes.
  const marker = renderer.createTextNode("");
  insertNode(pool, marker);
  let current: unknown;
  renderer.insert(pool, () => props.children, marker, undefined, {
    onUpdate: (value: unknown) => {
      current = value;
    },
  } as never);
  onCleanup(() => {
    const nodes = Array.isArray(current) ? current.flat(Infinity) : [current];
    for (const n of nodes) if (n && typeof n === "object" && (n as SolidNode).parent === pool) removeFromParent(n as SolidNode);
    if (marker.parent) removeFromParent(marker);
  });
  return undefined;
}

function setHidden(node: SolidNode, hidden: boolean): void {
  if (node.type === "#text" || node.type === "#root" || node.hidden === hidden) return;
  node.hidden = hidden;
  if (node.created) emit({ op: hidden ? "hide" : "unhide", id: node.id });
}

/// Keeps `children` mounted while `mode` is "hidden": their top-level widgets
/// are hidden in place instead of removed, so a hidden `<webview>` keeps its
/// page, scroll position and JS state, all of which unmounting destroys.
export function Activity(props: { mode: "visible" | "hidden"; children?: SolidElement }): SolidElement {
  const resolved = children(() => props.children);
  createRenderEffect(
    () => ({ hidden: props.mode === "hidden", nodes: resolved.toArray() }),
    ({ hidden, nodes }) => {
      for (const n of nodes) if (n && typeof n === "object") setHidden(n as unknown as SolidNode, hidden);
    },
  );
  return resolved as unknown as SolidElement;
}

interface Mounted {
  code: () => SolidElement;
  dispose: () => void;
}

declare global {
  // eslint-disable-next-line no-var
  var __nd_solid_mounted: Mounted | undefined;
  /** Set by register.ts under `nd dev`: the per-module hot context the refresh runtime drives. */
  // eslint-disable-next-line no-var
  var __nd_solid_hot: ((id: string) => unknown) | undefined;
  // eslint-disable-next-line no-var
  var __nd_solid_remount: (() => void) | undefined;
}

// Render-phase errors. One an <Errored> boundary renders a fallback for
// reaches Solid's client error hook and is reported, non-fatal. One no app
// boundary catches lands in the boundary wrapped around the whole tree, which
// is fatal regardless of policy: without it Solid halts reactivity and leaves
// a window that looks alive. The hook fires for that root boundary too,
// before its fallback runs, so a caught report waits a microtask and is
// dropped once the root has claimed the error.
const fatalRender = new WeakSet<object>();

function installRenderErrorHooks(): void {
  configureClientErrors({
    onError(error) {
      queueMicrotask(() => {
        if (typeof error === "object" && error !== null && fatalRender.has(error)) return;
        reportRenderError(error, "renderCaught");
      });
    },
  });
}

function guarded(code: () => SolidElement): () => SolidElement {
  return () =>
    renderer.createComponent(Errored as (props: Parameters<typeof Errored>[0]) => SolidNode, {
      fallback: (error: () => unknown) => {
        const raw = error();
        if (typeof raw === "object" && raw !== null) fatalRender.add(raw);
        reportRenderError(raw, "renderUncaught");
        return process.exit(1);
      },
      get children() {
        return code();
      },
    });
}

function mountRoot(code: () => SolidElement): void {
  globalThis.__nd_solid_mounted?.dispose();
  const root = makeNode("#root");
  globalThis.__nd_solid_mounted = { code, dispose: renderer.render(guarded(code) as () => SolidNode, root) };
}

/// Connects to the host and mounts `code`'s tree; the open host socket keeps
/// the process alive. It resolves rather than parking the entry: Bun adds a
/// module a plugin compiled to `--hot`'s watch set only once its evaluation
/// finishes, so a parked .tsx entry would never reload.
///
/// A `bun --hot` re-eval calls this again. Under `nd dev` the refresh runtime
/// (see register.ts) patches the live tree's components in place, so the
/// mounted tree, its native windows and the state of every unedited
/// component stay; only an edit the runtime cannot patch remounts from the
/// newest `code`. Without the refresh runtime a re-eval disposes and remounts.
export async function render(code: () => SolidElement): Promise<void> {
  installErrorHandlers();
  installRenderErrorHooks();
  session = await connect();
  session.flush = shipBatch;
  const mounted = globalThis.__nd_solid_mounted;
  if (mounted && globalThis.__nd_solid_hot) {
    mounted.code = code;
  } else {
    mountRoot(code);
  }
  globalThis.__nd_solid_remount = () => mountRoot(globalThis.__nd_solid_mounted!.code);
}
