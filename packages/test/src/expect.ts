// Polling assertions over a Locator, plus a small non-polling `expect` for
// plain values so a drive script needs only one assertion vocabulary.
//
// A locator matcher re-reads the tree every 100ms until it passes or the
// deadline runs out, so nothing in a drive has to sleep before asserting.
// `.not` inverts the predicate and keeps the polling: `.not.toBeVisible()`
// waits for the widget to go away rather than failing on the first read.
import { allNodes, matchText, nodeChecked, nodeName, nodePlaceholder, nodeText, renderValue, type NdNode } from "./matcher.ts";
import { rankCandidates, StrictModeError, TimeoutError } from "./errors.ts";
import { intendedName } from "./selectors.ts";
import { Locator } from "./locator.ts";

const POLL_MS = 100;

export interface ExpectOptions {
  timeout?: number;
}

interface Verdict {
  pass: boolean;
  actual: string;
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

function describe(expected: unknown): string {
  if (expected instanceof RegExp) return String(expected);
  return typeof expected === "string" ? JSON.stringify(expected) : String(expected);
}

function textMatches(expected: string | RegExp, actual: string, exact: boolean): boolean {
  return matchText(expected instanceof RegExp ? { regex: expected } : { value: expected, exact }, actual);
}

export class LocatorAssertions {
  constructor(
    private readonly locator: Locator,
    private readonly negated = false,
  ) {}

  get not(): LocatorAssertions {
    return new LocatorAssertions(this.locator, !this.negated);
  }

  private async run(
    name: string,
    expected: string,
    strict: boolean,
    evaluate: (matches: NdNode[]) => Verdict,
    opts: ExpectOptions,
  ): Promise<void> {
    const timeout = opts.timeout ?? this.locator.client.actionTimeout;
    const deadline = Date.now() + timeout;
    let root: NdNode | null = null;
    let count = 0;
    let actual = "";
    for (;;) {
      const snap = await this.locator.snapshot();
      root = snap.root;
      count = snap.matches.length;
      if (strict && count > 1) {
        throw new StrictModeError(this.locator.selector, snap.matches, root?.text ? JSON.stringify(root.text) : "?");
      }
      const verdict = evaluate(snap.matches);
      actual = verdict.actual;
      if (verdict.pass !== this.negated) return;
      if (Date.now() >= deadline) break;
      await sleep(POLL_MS);
    }
    const call = `expect(${this.locator.selector})${this.negated ? ".not" : ""}.${name}`;
    const extra = `Expected: ${this.negated ? "not " : ""}${expected}\nReceived: ${actual}`;
    const log = [`waiting for ${this.locator.selector}`, `resolved ${count} element${count === 1 ? "" : "s"}`];
    const candidates = count === 0 && root ? rankCandidates(allNodes(root), intendedName(this.locator.parts)) : [];
    throw new TimeoutError(call, timeout, log, candidates, root?.text ? JSON.stringify(root.text) : "?", extra);
  }

  toBeVisible(opts: ExpectOptions = {}): Promise<void> {
    return this.run("toBeVisible", "visible", true, (m) => ({
      pass: m.length > 0 && m[0]!.visible,
      actual: m.length ? (m[0]!.visible ? "visible" : "hidden") : "not attached",
    }), opts);
  }

  toBeHidden(opts: ExpectOptions = {}): Promise<void> {
    return this.run("toBeHidden", "hidden", true, (m) => ({
      pass: m.length === 0 || !m[0]!.visible,
      actual: m.length ? (m[0]!.visible ? "visible" : "hidden") : "not attached",
    }), opts);
  }

  toBeAttached(opts: ExpectOptions = {}): Promise<void> {
    return this.run("toBeAttached", "attached", false, (m) => ({
      pass: m.length > 0,
      actual: `${m.length} element${m.length === 1 ? "" : "s"}`,
    }), opts);
  }

  toBeEnabled(opts: ExpectOptions = {}): Promise<void> {
    return this.run("toBeEnabled", "enabled", true, (m) => ({
      pass: m.length > 0 && m[0]!.enabled,
      actual: m.length ? (m[0]!.enabled ? "enabled" : "disabled") : "not attached",
    }), opts);
  }

  toBeDisabled(opts: ExpectOptions = {}): Promise<void> {
    return this.run("toBeDisabled", "disabled", true, (m) => ({
      pass: m.length > 0 && !m[0]!.enabled,
      actual: m.length ? (m[0]!.enabled ? "enabled" : "disabled") : "not attached",
    }), opts);
  }

  toBeChecked(opts: ExpectOptions & { checked?: boolean } = {}): Promise<void> {
    const want = opts.checked ?? true;
    return this.run("toBeChecked", String(want), true, (m) => ({
      pass: m.length > 0 && nodeChecked(m[0]!) === want,
      actual: m.length ? String(nodeChecked(m[0]!)) : "not attached",
    }), opts);
  }

  toBeFocused(opts: ExpectOptions = {}): Promise<void> {
    return this.run("toBeFocused", "focused", true, (m) => ({
      pass: m.length > 0 && m[0]!.focused,
      actual: m.length ? (m[0]!.focused ? "focused" : "not focused") : "not attached",
    }), opts);
  }

  toHaveText(expected: string | RegExp, opts: ExpectOptions = {}): Promise<void> {
    return this.run("toHaveText", describe(expected), true, (m) => ({
      pass: m.length > 0 && textMatches(expected, nodeText(m[0]!), true),
      actual: m.length ? JSON.stringify(nodeText(m[0]!)) : "not attached",
    }), opts);
  }

  toContainText(expected: string | RegExp, opts: ExpectOptions = {}): Promise<void> {
    return this.run("toContainText", describe(expected), true, (m) => ({
      pass: m.length > 0 && textMatches(expected, nodeText(m[0]!), false),
      actual: m.length ? JSON.stringify(nodeText(m[0]!)) : "not attached",
    }), opts);
  }

  toHaveValue(expected: string | number | boolean | RegExp, opts: ExpectOptions = {}): Promise<void> {
    const want = expected instanceof RegExp ? expected : renderValue(expected);
    return this.run("toHaveValue", describe(want), true, (m) => ({
      pass: m.length > 0 && textMatches(want, renderValue(m[0]!.value), true),
      actual: m.length ? JSON.stringify(renderValue(m[0]!.value)) : "not attached",
    }), opts);
  }

  toHaveCount(expected: number, opts: ExpectOptions = {}): Promise<void> {
    return this.run("toHaveCount", String(expected), false, (m) => ({
      pass: m.length === expected,
      actual: String(m.length),
    }), opts);
  }

  toHaveAttribute(name: string, expected?: string | RegExp, opts: ExpectOptions = {}): Promise<void> {
    const read = (node: NdNode): string | null => {
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
    };
    return this.run("toHaveAttribute", expected === undefined ? `${name} present` : `${name}=${describe(expected)}`, true, (m) => {
      const got = m.length ? read(m[0]!) : null;
      const pass = expected === undefined ? got !== null : got !== null && textMatches(expected, got, true);
      return { pass, actual: got === null ? "null" : JSON.stringify(got) };
    }, opts);
  }
}

function deepEqual(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const ka = Object.keys(a as object);
  const kb = Object.keys(b as object);
  if (ka.length !== kb.length) return false;
  return ka.every((k) => deepEqual((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]));
}

export class ValueAssertions {
  constructor(
    private readonly actual: unknown,
    private readonly negated = false,
  ) {}

  get not(): ValueAssertions {
    return new ValueAssertions(this.actual, !this.negated);
  }

  private assert(name: string, pass: boolean, expected: string): void {
    if (pass !== this.negated) return;
    throw new Error(
      `expect(${describe(this.actual)})${this.negated ? ".not" : ""}.${name}\n` +
        `Expected: ${this.negated ? "not " : ""}${expected}\nReceived: ${describe(this.actual)}`,
    );
  }

  toBe(expected: unknown): void {
    this.assert("toBe", Object.is(this.actual, expected), describe(expected));
  }

  toEqual(expected: unknown): void {
    this.assert("toEqual", deepEqual(this.actual, expected), describe(expected));
  }

  toBeTruthy(): void {
    this.assert("toBeTruthy", Boolean(this.actual), "truthy");
  }

  toBeFalsy(): void {
    this.assert("toBeFalsy", !this.actual, "falsy");
  }

  toBeNull(): void {
    this.assert("toBeNull", this.actual === null, "null");
  }

  toBeUndefined(): void {
    this.assert("toBeUndefined", this.actual === undefined, "undefined");
  }

  toContain(expected: unknown): void {
    const pass = Array.isArray(this.actual)
      ? this.actual.includes(expected)
      : typeof this.actual === "string" && typeof expected === "string" && this.actual.includes(expected);
    this.assert("toContain", pass, describe(expected));
  }

  toMatch(expected: RegExp | string): void {
    const actual = String(this.actual);
    const pass = expected instanceof RegExp ? expected.test(actual) : actual.includes(expected);
    this.assert("toMatch", pass, describe(expected));
  }

  toBeGreaterThan(expected: number): void {
    this.assert("toBeGreaterThan", Number(this.actual) > expected, `> ${expected}`);
  }

  toBeGreaterThanOrEqual(expected: number): void {
    this.assert("toBeGreaterThanOrEqual", Number(this.actual) >= expected, `>= ${expected}`);
  }

  toBeLessThan(expected: number): void {
    this.assert("toBeLessThan", Number(this.actual) < expected, `< ${expected}`);
  }

  toBeLessThanOrEqual(expected: number): void {
    this.assert("toBeLessThanOrEqual", Number(this.actual) <= expected, `<= ${expected}`);
  }
}

export function expect(actual: Locator): LocatorAssertions;
export function expect(actual: unknown): ValueAssertions;
export function expect(actual: unknown): LocatorAssertions | ValueAssertions {
  return actual instanceof Locator ? new LocatorAssertions(actual) : new ValueAssertions(actual);
}
