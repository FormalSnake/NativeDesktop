// Prop semantics every renderer has to share, so the host sees the same op
// stream whichever one built the tree.

import type { Handler } from "./ops.ts";
import { widgetEvents, handlerPropNames, widgetPlatforms } from "./generated/schema-meta.ts";
import type { WidgetType } from "./generated/widgets.ts";
import { Platform } from "./platform.ts";
import { isHot } from "./session.ts";

// A function-valued `on*` prop the schema does not declare is dropped on the
// floor: collectHandlers reads only declared names, so `onClicked` on a
// <button> (whose event prop is `onClick`) never registers a listener while
// the click RPC still answers dispatched:true and the GTK signal is still
// connected. Nothing else in the stack notices, because examples/ and app
// trees are not covered by any tsconfig, so the JSX type error never runs.
// Warn once per type+prop.
const warnedUnknownHandler = new Set<string>();

export function warnUnknownHandler(type: string, key: string, value: unknown): void {
  if (typeof value !== "function") return;
  if (key.length < 3 || !key.startsWith("on") || key[2]! !== key[2]!.toUpperCase()) return;
  const declared = handlerPropNames[type] ?? [];
  if (declared.includes(key)) return;
  const seen = `${type}.${key}`;
  if (warnedUnknownHandler.has(seen)) return;
  warnedUnknownHandler.add(seen);
  const hint = declared.length ? `Declared events: ${declared.join(", ")}.` : "This widget declares no events.";
  console.warn(`ND_WARN <${type}> has no "${key}" event, so the handler will never fire. ${hint}`);
}

/** Event name -> handler, read from the props the schema declares as events. */
export function collectHandlers(type: string, props: Record<string, unknown>): Record<string, Handler> {
  for (const key of Object.keys(props)) warnUnknownHandler(type, key, props[key]);
  const out: Record<string, Handler> = {};
  for (const ev of widgetEvents[type] ?? []) {
    const h = props[ev.handler];
    if (typeof h === "function") out[ev.name] = h as Handler;
  }
  return out;
}

/** The wire event name a handler prop (`onClick`) listens to, if the schema declares it. */
export function eventForHandler(type: string, prop: string): string | undefined {
  for (const ev of widgetEvents[type] ?? []) if (ev.handler === prop) return ev.name;
  return undefined;
}

/** Wire id behind a ref-valued prop, a node handle. */
export function refTargetId(v: unknown): number | undefined {
  if (v === null || typeof v !== "object") return undefined;
  const id = (v as { id?: unknown }).id;
  return typeof id === "number" ? id : undefined;
}

// How deep propsEqual walks before it gives up and answers "changed". Covers
// StyleProp (an object of scalars plus font/padding/margin/border sub-objects)
// and the array-of-record props (`rows`, `columns`, `nodes`) without ever
// recursing into something unbounded.
const PROP_COMPARE_DEPTH = 4;

function isPlainObject(v: unknown): v is Record<string, unknown> {
  if (typeof v !== "object" || v === null) return false;
  const proto = Object.getPrototypeOf(v);
  return proto === Object.prototype || proto === null;
}

/// Structural equality for prop values, bounded by PROP_COMPARE_DEPTH.
/// A JSX literal (`style={{...}}`, `rows={items.map(...)}`) is a fresh object
/// on every render, so identity comparison reports every one of them as
/// changed and ships a full `update` op for a prop nobody touched. Anything
/// that is not a plain object or an array (functions, class instances, Dates)
/// falls back to identity, as does hitting the depth cap: the conservative
/// answer is "changed", which costs a redundant update, never a missed one.
export function propsEqual(a: unknown, b: unknown, depth = 0): boolean {
  if (Object.is(a, b)) return true;
  if (depth >= PROP_COMPARE_DEPTH) return false;
  if (Array.isArray(a)) {
    if (!Array.isArray(b) || a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (!propsEqual(a[i], b[i], depth + 1)) return false;
    return true;
  }
  if (!isPlainObject(a) || !isPlainObject(b)) return false;
  const keys = Object.keys(a);
  if (keys.length !== Object.keys(b).length) return false;
  for (const k of keys) {
    if (!Object.prototype.hasOwnProperty.call(b, k)) return false;
    if (!propsEqual(a[k], b[k], depth + 1)) return false;
  }
  return true;
}

/// The two universal props whose removal marker is not null. Both are applied
/// by a set-replace pass over the whole value (Backend.swift's `ndApplyStyle`,
/// src/gtk/style.zig's per-node CSS provider and class allowlist), so the empty
/// value IS the reset; a null would only fall through their type guards.
const REMOVAL_VALUE: Record<string, unknown> = {
  style: Object.freeze({}),
  cssClasses: Object.freeze([]),
};

/// What a dropped prop is sent as. NDP has no removal tag (an `update` op
/// carries a props object, nothing else), so null is the removal marker: it is
/// the one value that cannot be a legitimate typed prop, and both encodings
/// carry it (JSON null, binary value tag 0x00). The generated appliers turn it
/// back into the prop's schema default (ndApplyDroppedDefaults, tools/codegen.ts).
export function removalValue(key: string): unknown {
  return REMOVAL_VALUE[key] ?? null;
}

const PLATFORM_LABEL: Record<"macos" | "linux", string> = { macos: "macOS", linux: "Linux" };

// One-time-per-type dev warning when an intrinsic mounts on a platform its
// schema entry doesn't list (e.g. <trayitem> on linux). It's not an error,
// the widget just renders as an invisible no-op there, but silently doing
// nothing is easy to miss. Gated on isHot() so a `nd build` release never
// pays for or prints this check.
const warnedPlatformMismatch = new Set<WidgetType>();
export function checkPlatform(type: WidgetType): void {
  if (!isHot() || warnedPlatformMismatch.has(type)) return;
  const allowed = widgetPlatforms[type];
  if (!allowed || (allowed as readonly string[]).includes(Platform.os)) return;
  warnedPlatformMismatch.add(type);
  const label = allowed.map((p) => PLATFORM_LABEL[p]).join("/");
  console.warn(`<${type}> is ${label}-only; it renders nothing on ${Platform.os}. Gate it with Platform.os.`);
}
