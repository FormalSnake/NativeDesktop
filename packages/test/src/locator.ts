// Playwright-shaped locators over the automation socket.
//
// A Locator is lazy: it holds a client, an optional window scope, and a list
// of selector parts. Nothing is resolved until an action or a reader runs,
// and every one of those re-resolves from a fresh getTree, so a locator kept
// across a re-render still points at the right widget.
//
// Actionability before a single-target action, in order: resolve (poll every
// 100ms until the deadline), refuse more than one match outright, require the
// one match to be visible, enabled and to occupy real pixels, then require
// its rectangle to be identical on the next read so a mid-animation widget is
// never clicked. A dispatch that comes back -32001 (the ref went stale
// between resolve and dispatch) goes around the loop again.
import { AutomationRpcError } from "./socket.ts";
import { RPC_ERRORS, type GetTreeResult, type ScreenshotResult } from "@nativedesktop/react/rpc";
import {
  allNodes,
  asNdNode,
  hasRealSize,
  nodeChecked,
  nodeName,
  nodePlaceholder,
  nodeText,
  renderValue,
  selectNodes,
  type NdNode,
} from "./matcher.ts";
import { formatSelector, intendedName, parseSelector, type SelectorPart, type TextSpec } from "./selectors.ts";
import { LocatorError, rankCandidates, StrictModeError, TimeoutError } from "./errors.ts";
import { Keyboard, Mouse, toChord } from "./keyboard.ts";

const POLL_MS = 100;

/** What a Locator needs from a host connection. AppHandle implements it; a
 * unit test can implement it over a canned tree. */
export interface LocatorClient {
  tree(window?: number): Promise<GetTreeResult>;
  callRpc(method: string, params?: Record<string, unknown>): Promise<unknown>;
  /** Default deadline for actions, readers and expect matchers. */
  actionTimeout: number;
}

export interface RoleOptions {
  name?: string | RegExp;
  exact?: boolean;
  checked?: boolean;
  disabled?: boolean;
}

export interface TextOptions {
  exact?: boolean;
}

export interface ActionOptions {
  timeout?: number;
}

export type WaitForState = "attached" | "detached" | "visible" | "hidden";

export interface BoundingBox {
  x: number;
  y: number;
  width: number;
  height: number;
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

function textSpec(text: string | RegExp, exact?: boolean): TextSpec {
  return text instanceof RegExp ? { regex: text } : { value: text, exact: exact ?? false };
}

function isStaleRef(e: unknown): boolean {
  return e instanceof AutomationRpcError && e.code === RPC_ERRORS.notActionable.code;
}

/** Calls a host method, turning "method not found" into a message that names
 * the missing RPC instead of a bare -32601. */
export async function callHost(client: LocatorClient, method: string, params: Record<string, unknown>): Promise<unknown> {
  try {
    return await client.callRpc(method, params);
  } catch (e) {
    if (e instanceof AutomationRpcError && e.code === RPC_ERRORS.methodNotFound.code) {
      throw new LocatorError(`host predates the "${method}" RPC`);
    }
    throw e;
  }
}

function notActionable(node: NdNode): string {
  if (!node.visible) return "not visible";
  if (!node.enabled) return "disabled";
  if (!hasRealSize(node)) {
    const g = node.geometry;
    return g ? `zero size (${g.w}x${g.h})` : "no geometry";
  }
  return "";
}

function sameRect(a: NdNode, b: NdNode): boolean {
  const x = a.geometry;
  const y = b.geometry;
  if (!x || !y) return x === y;
  return x.x === y.x && x.y === y.y && x.w === y.w && x.h === y.h;
}

interface ResolveOptions {
  action: string;
  timeout?: number;
  actionable?: boolean;
  stable?: boolean;
}

export class Locator {
  constructor(
    readonly client: LocatorClient,
    readonly parts: SelectorPart[],
    readonly windowRef?: number,
  ) {}

  get selector(): string {
    return formatSelector(this.parts);
  }

  toString(): string {
    return this.selector;
  }

  private derive(extra: SelectorPart[]): Locator {
    return new Locator(this.client, [...this.parts, ...extra], this.windowRef);
  }

  // --- chaining ------------------------------------------------------------

  locator(selector: string | Locator): Locator {
    return this.derive(typeof selector === "string" ? parseSelector(selector) : selector.parts);
  }

  getByTestId(testId: string): Locator {
    return this.derive([{ kind: "testid", value: testId }]);
  }

  getByRole(role: string, opts: RoleOptions = {}): Locator {
    return this.derive([roleSpec(role, opts)]);
  }

  getByText(text: string | RegExp, opts: TextOptions = {}): Locator {
    return this.derive([{ kind: "text", ...textSpec(text, opts.exact) }]);
  }

  getByLabel(text: string | RegExp, opts: TextOptions = {}): Locator {
    return this.derive([{ kind: "label", ...textSpec(text, opts.exact) }]);
  }

  getByPlaceholder(text: string | RegExp, opts: TextOptions = {}): Locator {
    return this.derive([{ kind: "placeholder", ...textSpec(text, opts.exact) }]);
  }

  filter(opts: { hasText?: string | RegExp; has?: Locator; hasNot?: Locator }): Locator {
    const extra: SelectorPart[] = [];
    if (opts.hasText !== undefined) extra.push({ kind: "has-text", ...textSpec(opts.hasText) });
    if (opts.has) extra.push({ kind: "has", parts: opts.has.parts });
    if (opts.hasNot) extra.push({ kind: "has-not", parts: opts.hasNot.parts });
    return this.derive(extra);
  }

  and(other: Locator): Locator {
    return this.derive([{ kind: "and", parts: other.parts }]);
  }

  first(): Locator {
    return this.derive([{ kind: "nth", index: 0 }]);
  }

  last(): Locator {
    return this.derive([{ kind: "nth", index: -1 }]);
  }

  nth(index: number): Locator {
    return this.derive([{ kind: "nth", index }]);
  }

  // --- resolution ----------------------------------------------------------

  /** One tree read, no waiting. The window scope going away reads as an
   * empty match set, the same as a selector that matches nothing. */
  async snapshot(): Promise<{ root: NdNode | null; matches: NdNode[] }> {
    let tree: GetTreeResult;
    try {
      tree = await this.client.tree(this.windowRef);
    } catch (e) {
      if (e instanceof AutomationRpcError && e.code === RPC_ERRORS.windowGone.code) return { root: null, matches: [] };
      throw e;
    }
    const root = asNdNode(tree.root);
    return { root, matches: selectNodes(root, this.parts) };
  }

  private async windowLabel(root: NdNode | null): Promise<string> {
    const ref = this.windowRef ?? root?.ref;
    try {
      const res = (await this.client.callRpc("windows")) as { windows: { ref: number; title: string | null }[] };
      const info = res.windows.find((w) => w.ref === ref) ?? res.windows[0];
      if (info?.title) return JSON.stringify(info.title);
    } catch {
      // The host is gone, or answered nothing useful; the root node still names it.
    }
    return root?.text ? JSON.stringify(root.text) : `#${ref ?? "?"}`;
  }

  private async stableFrame(node: NdNode): Promise<NdNode | null> {
    await sleep(POLL_MS);
    const { matches } = await this.snapshot();
    const again = matches.find((m) => m.ref === node.ref);
    return again && sameRect(node, again) ? again : null;
  }

  private async resolveOne(opts: ResolveOptions): Promise<NdNode> {
    const timeout = opts.timeout ?? this.client.actionTimeout;
    const deadline = Date.now() + timeout;
    let root: NdNode | null = null;
    let count = 0;
    let reason = "";
    for (;;) {
      const snap = await this.snapshot();
      root = snap.root;
      count = snap.matches.length;
      if (count > 1) throw new StrictModeError(this.selector, snap.matches, await this.windowLabel(root));
      const node = snap.matches[0];
      if (node) {
        if (!opts.actionable) return node;
        reason = notActionable(node);
        if (!reason) {
          if (!opts.stable) return node;
          const settled = await this.stableFrame(node);
          if (settled) return settled;
          reason = "geometry never settled";
        }
      }
      if (Date.now() >= deadline) break;
      await sleep(POLL_MS);
    }
    const log = [`waiting for ${this.selector}`, `resolved ${count} element${count === 1 ? "" : "s"}`];
    if (reason) log.push(`element is ${reason}`);
    const candidates = root ? rankCandidates(allNodes(root), intendedName(this.parts)) : [];
    throw new TimeoutError(`locator.${opts.action}`, timeout, log, candidates, await this.windowLabel(root));
  }

  /** Resolved node, waiting only for it to exist. */
  node(opts: ActionOptions = {}): Promise<NdNode> {
    return this.resolveOne({ action: "node", timeout: opts.timeout });
  }

  /** Resolved wire ref, for the coordinate RPCs a locator does not wrap. */
  async ref(opts: ActionOptions = {}): Promise<number> {
    return (await this.node(opts)).ref;
  }

  private async dispatch<T>(action: string, run: (ref: number) => Promise<T>, opts: ActionOptions): Promise<T> {
    const timeout = opts.timeout ?? this.client.actionTimeout;
    const deadline = Date.now() + timeout;
    for (;;) {
      const node = await this.resolveOne({
        action,
        timeout: Math.max(POLL_MS, deadline - Date.now()),
        actionable: true,
        stable: true,
      });
      try {
        return await run(node.ref);
      } catch (e) {
        if (isStaleRef(e) && Date.now() < deadline) continue;
        throw e;
      }
    }
  }

  // --- actions -------------------------------------------------------------

  /** `action` names a SourceTree row action, dispatched on the row the
   * locator resolves to. */
  async click(opts: ActionOptions & { action?: string } = {}): Promise<void> {
    await this.dispatch("click", (ref) => this.client.callRpc("click", { ref, action: opts.action }), opts);
  }

  async dblclick(opts: ActionOptions = {}): Promise<void> {
    await this.dispatch("dblclick", (ref) => this.client.callRpc("doubleClick", { ref }), opts);
  }

  async rightClick(opts: ActionOptions = {}): Promise<void> {
    await this.dispatch("rightClick", (ref) => this.client.callRpc("rightClick", { ref }), opts);
  }

  async hover(opts: ActionOptions = {}): Promise<void> {
    await this.dispatch("hover", (ref) => this.client.callRpc("hover", { ref }), opts);
  }

  /** Replaces the widget's value, the way a user finishing an edit would.
   * Kind-dispatched host-side: string for text, number for a slider. */
  async fill(value: string | number | boolean, opts: ActionOptions = {}): Promise<void> {
    await this.dispatch("fill", (ref) => this.client.callRpc("setValue", { ref, value }), opts);
  }

  /** Appends text without clearing what is there, the `type` RPC's semantics. */
  async type(text: string, opts: ActionOptions = {}): Promise<void> {
    await this.dispatch("type", (ref) => this.client.callRpc("type", { ref, text }), opts);
  }

  pressSequentially(text: string, opts: ActionOptions = {}): Promise<void> {
    return this.type(text, opts);
  }

  /** Focuses the widget, then sends one chord in Playwright key names. */
  async press(key: string, opts: ActionOptions = {}): Promise<void> {
    await this.focus(opts);
    await this.client.callRpc("keys", { keys: toChord(key), window: this.windowRef });
  }

  check(opts: ActionOptions = {}): Promise<void> {
    return this.setChecked(true, opts);
  }

  uncheck(opts: ActionOptions = {}): Promise<void> {
    return this.setChecked(false, opts);
  }

  async setChecked(checked: boolean, opts: ActionOptions = {}): Promise<void> {
    const node = await this.resolveOne({ action: "setChecked", timeout: opts.timeout, actionable: true, stable: true });
    if (nodeChecked(node) === checked) return;
    await this.dispatch("setChecked", (ref) => this.client.callRpc("setValue", { ref, value: checked }), opts);
  }

  /** An index selects that row. A string is looked up in the node's
   * `options` first, so a drive can name the option instead of counting to
   * it, and is passed through when the host sends no options (Select and
   * ComboBox both accept a literal). */
  async selectOption(value: string | number, opts: ActionOptions = {}): Promise<void> {
    let resolved: string | number = value;
    if (typeof value === "string") {
      const node = await this.resolveOne({ action: "selectOption", timeout: opts.timeout });
      const index = node.options?.indexOf(value) ?? -1;
      if (index >= 0) resolved = index;
    }
    await this.dispatch("selectOption", (ref) => this.client.callRpc("setValue", { ref, value: resolved }), opts);
  }

  async focus(opts: ActionOptions = {}): Promise<void> {
    await this.dispatch("focus", (ref) => callHost(this.client, "focus", { ref }), opts);
  }

  /** There is no blur RPC: focus moves to the enclosing window instead, which
   * is what resigns a widget's first-responder status. */
  async blur(opts: ActionOptions = {}): Promise<void> {
    const node = await this.resolveOne({ action: "blur", timeout: opts.timeout });
    const { root } = await this.snapshot();
    const target = this.windowRef ?? root?.ref;
    if (target === undefined) throw new LocatorError(`blur: no window to move focus to (from ref ${node.ref})`);
    await callHost(this.client, "focus", { ref: target });
  }

  async scrollIntoViewIfNeeded(opts: ActionOptions = {}): Promise<void> {
    const node = await this.resolveOne({ action: "scrollIntoViewIfNeeded", timeout: opts.timeout });
    await callHost(this.client, "scrollIntoView", { ref: node.ref });
  }

  async dragTo(
    target: Locator,
    opts: ActionOptions & { steps?: number; durationMs?: number } = {},
  ): Promise<void> {
    const toRef = await target.resolveOne({ action: "dragTo", timeout: opts.timeout, actionable: true, stable: true });
    await this.dispatch(
      "dragTo",
      (ref) =>
        this.client.callRpc("drag", {
          fromRef: ref,
          toRef: toRef.ref,
          steps: opts.steps,
          durationMs: opts.durationMs,
          window: this.windowRef,
        }),
      opts,
    );
  }

  /** Renders this node's own widget to a PNG. `path` is optional: without
   * one the host writes beside the automation socket and answers where. The
   * result's width/height are the node's pixels, not the window's. */
  async screenshot(path?: string, opts: ActionOptions = {}): Promise<ScreenshotResult> {
    const node = await this.resolveOne({ action: "screenshot", timeout: opts.timeout });
    return (await callHost(this.client, "snapshotNode", { ref: node.ref, path })) as ScreenshotResult;
  }

  // --- readers -------------------------------------------------------------

  async boundingBox(opts: ActionOptions = {}): Promise<BoundingBox | null> {
    const g = (await this.node(opts)).geometry;
    return g ? { x: g.x, y: g.y, width: g.w, height: g.h } : null;
  }

  async textContent(opts: ActionOptions = {}): Promise<string | null> {
    return (await this.node(opts)).text;
  }

  async inputValue(opts: ActionOptions = {}): Promise<string> {
    return renderValue((await this.node(opts)).value);
  }

  /** Node fields by name: testID, role, type, text, label, placeholder,
   * value. Anything else answers null rather than throwing, matching the
   * "attribute is absent" reading. */
  async getAttribute(name: string, opts: ActionOptions = {}): Promise<string | null> {
    const node = await this.node(opts);
    switch (name) {
      case "testId":
      case "testID":
        return node.testID;
      case "role":
        return node.role;
      case "type":
        return node.type;
      case "text":
        return node.text;
      case "label":
        return nodeName(node) || null;
      case "placeholder":
        return nodePlaceholder(node) || null;
      case "value":
        return node.value === null || node.value === undefined ? null : renderValue(node.value);
      default:
        return null;
    }
  }

  async isVisible(): Promise<boolean> {
    const { matches } = await this.snapshot();
    return matches.length > 0 && matches[0]!.visible;
  }

  async isEnabled(opts: ActionOptions = {}): Promise<boolean> {
    return (await this.node(opts)).enabled;
  }

  async isChecked(opts: ActionOptions = {}): Promise<boolean> {
    return nodeChecked(await this.node(opts)) === true;
  }

  async isFocused(opts: ActionOptions = {}): Promise<boolean> {
    return (await this.node(opts)).focused;
  }

  async count(): Promise<number> {
    return (await this.snapshot()).matches.length;
  }

  async all(): Promise<Locator[]> {
    const n = await this.count();
    return Array.from({ length: n }, (_, i) => this.nth(i));
  }

  async allTextContents(): Promise<string[]> {
    return (await this.snapshot()).matches.map(nodeText);
  }

  async waitFor(opts: ActionOptions & { state?: WaitForState } = {}): Promise<void> {
    const state = opts.state ?? "visible";
    const timeout = opts.timeout ?? this.client.actionTimeout;
    const deadline = Date.now() + timeout;
    let root: NdNode | null = null;
    let count = 0;
    for (;;) {
      const snap = await this.snapshot();
      root = snap.root;
      count = snap.matches.length;
      const visible = snap.matches.filter((m) => m.visible).length;
      const ok =
        state === "attached"
          ? count > 0
          : state === "detached"
            ? count === 0
            : state === "visible"
              ? visible > 0
              : visible === 0;
      if (ok) return;
      if (Date.now() >= deadline) break;
      await sleep(POLL_MS);
    }
    const log = [`waiting for ${this.selector} to be ${state}`, `resolved ${count} element${count === 1 ? "" : "s"}`];
    const candidates = root ? rankCandidates(allNodes(root), intendedName(this.parts)) : [];
    throw new TimeoutError("locator.waitFor", timeout, log, candidates, await this.windowLabel(root));
  }
}

function roleSpec(role: string, opts: RoleOptions): SelectorPart {
  const part: SelectorPart = { kind: "role", role };
  if (opts.name !== undefined) part.name = textSpec(opts.name, opts.exact);
  if (opts.checked !== undefined) part.checked = opts.checked;
  if (opts.disabled !== undefined) part.disabled = opts.disabled;
  return part;
}

/** The getBy* surface, bound to a client and optionally to one window. */
export class LocatorFactory {
  readonly keyboard: Keyboard;
  readonly mouse: Mouse;

  constructor(
    readonly client: LocatorClient,
    readonly windowRef?: number,
  ) {
    this.keyboard = new Keyboard(client, windowRef);
    this.mouse = new Mouse(client, windowRef);
  }

  locator(selector: string): Locator {
    return new Locator(this.client, parseSelector(selector), this.windowRef);
  }

  getByTestId(testId: string): Locator {
    return new Locator(this.client, [{ kind: "testid", value: testId }], this.windowRef);
  }

  getByRole(role: string, opts: RoleOptions = {}): Locator {
    return new Locator(this.client, [roleSpec(role, opts)], this.windowRef);
  }

  getByText(text: string | RegExp, opts: TextOptions = {}): Locator {
    return new Locator(this.client, [{ kind: "text", ...textSpec(text, opts.exact) }], this.windowRef);
  }

  getByLabel(text: string | RegExp, opts: TextOptions = {}): Locator {
    return new Locator(this.client, [{ kind: "label", ...textSpec(text, opts.exact) }], this.windowRef);
  }

  getByPlaceholder(text: string | RegExp, opts: TextOptions = {}): Locator {
    return new Locator(this.client, [{ kind: "placeholder", ...textSpec(text, opts.exact) }], this.windowRef);
  }
}
