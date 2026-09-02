// Locator, expect and snapshot against a fake client, so the actionability
// loop, the strict-mode refusal, the stale-ref retry and the polling matchers
// are covered without a host process.
import { expect as bunExpect, test } from "bun:test";
import type { GetTreeResult, JsonNode } from "@nativedesktop/react/rpc";
import fixture from "./fixtures/gestures-tree.json" with { type: "json" };
import { AutomationRpcError } from "./socket.ts";
import { Locator, LocatorError, LocatorFactory, StrictModeError, TimeoutError, type LocatorClient } from "./index.ts";
import { expect } from "./expect.ts";
import { renderSnapshot } from "./snapshot.ts";
import { asNdNode } from "./matcher.ts";
import { toChord } from "./keyboard.ts";

interface NodeSpec {
  ref: number;
  type: string;
  role?: string;
  testID?: string;
  text?: string;
  value?: unknown;
  visible?: boolean;
  enabled?: boolean;
  focused?: boolean;
  checked?: boolean;
  selected?: boolean;
  expanded?: boolean;
  placeholder?: string;
  label?: string;
  options?: string[];
  w?: number;
  h?: number;
  children?: NodeSpec[];
}

function node(spec: NodeSpec): JsonNode {
  return {
    ref: spec.ref,
    type: spec.type,
    testID: spec.testID ?? null,
    text: spec.text ?? null,
    visible: spec.visible ?? true,
    geometry: { x: 0, y: 0, w: spec.w ?? 100, h: spec.h ?? 20 },
    children: (spec.children ?? []).map(node),
    itemCount: null,
    rows: null,
    role: spec.role ?? null,
    enabled: spec.enabled ?? true,
    focused: spec.focused ?? false,
    value: spec.value ?? null,
    checked: spec.checked ?? null,
    selected: spec.selected ?? null,
    expanded: spec.expanded ?? null,
    placeholder: spec.placeholder ?? null,
    label: spec.label ?? null,
    options: spec.options ?? null,
  };
}

function tree(root: NodeSpec): GetTreeResult {
  return { coordinateSpace: "logical-window-topleft", root: node(root) };
}

class FakeClient implements LocatorClient {
  actionTimeout = 600;
  readonly calls: { method: string; params?: Record<string, unknown> }[] = [];
  reads = 0;
  /** Rejections queued per method, consumed one per call. */
  readonly failures = new Map<string, Error[]>();

  constructor(private readonly snapshots: (read: number) => GetTreeResult) {}

  tree(): Promise<GetTreeResult> {
    return Promise.resolve(this.snapshots(this.reads++));
  }

  callRpc(method: string, params?: Record<string, unknown>): Promise<unknown> {
    this.calls.push({ method, params });
    const queued = this.failures.get(method)?.shift();
    if (queued) return Promise.reject(queued);
    if (method === "windows") return Promise.resolve({ windows: [{ ref: 1, title: "Notes" }] });
    return Promise.resolve({ ref: params?.ref, dispatched: true });
  }

  fail(method: string, error: Error): void {
    const list = this.failures.get(method) ?? [];
    list.push(error);
    this.failures.set(method, list);
  }
}

const twoButtons = tree({
  ref: 1,
  type: "Window",
  role: "window",
  text: "Notes",
  children: [
    { ref: 2, type: "Button", role: "button", testID: "save", text: "Save" },
    { ref: 3, type: "Button", role: "button", testID: "save-as", text: "Save As" },
    { ref: 4, type: "Checkbox", role: "checkbox", testID: "agree", text: "I agree", value: false },
  ],
});

/** The error a promise rejects with, so a test can assert on its text. */
async function rejection(promise: Promise<unknown>): Promise<Error> {
  try {
    await promise;
  } catch (e) {
    return e as Error;
  }
  throw new Error("expected a rejection");
}

function factory(snapshots: (read: number) => GetTreeResult): { app: FakeClient; page: LocatorFactory } {
  const app = new FakeClient(snapshots);
  return { app, page: new LocatorFactory(app) };
}

test("click resolves by testID and dispatches on the ref", async () => {
  const { app, page } = factory(() => twoButtons);
  await page.getByTestId("save").click();
  bunExpect(app.calls).toEqual([{ method: "click", params: { ref: 2, action: undefined } }]);
});

test("two matches for a single-target action fail immediately", async () => {
  const { app, page } = factory(() => twoButtons);
  const started = Date.now();
  const err = await rejection(page.getByRole("button").click());
  bunExpect(err).toBeInstanceOf(StrictModeError);
  bunExpect(err.message).toContain("resolved to 2 elements");
  bunExpect(err.message).toContain(".first()");
  bunExpect(Date.now() - started).toBeLessThan(app.actionTimeout);
});

test("first/nth/filter narrow a strict-mode violation", async () => {
  const { app, page } = factory(() => twoButtons);
  await page.getByRole("button").first().click();
  await page.getByRole("button").nth(1).click();
  await page.getByRole("button").filter({ hasText: "Save As" }).click();
  bunExpect(app.calls.map((c) => c.params?.ref)).toEqual([2, 3, 3]);
});

test("a timeout names the call, the log and the nearest candidates", async () => {
  const { page } = factory(() => asFixture());
  const err = await rejection(page.getByTestId("volume-slidr").click({ timeout: 250 }));
  bunExpect(err).toBeInstanceOf(TimeoutError);
  bunExpect(err.message).toContain("locator.click: Timeout 250ms exceeded.");
  bunExpect(err.message).toContain("waiting for testid=volume-slidr");
  bunExpect(err.message).toContain("resolved 0 elements");
  bunExpect(err.message).toContain('Nearest candidates in window "Notes"');
  bunExpect(err.message).toContain("volume-slider");
});

test("an unactionable match reports why it was skipped", async () => {
  const { page } = factory(() =>
    tree({ ref: 1, type: "Window", role: "window", children: [{ ref: 2, type: "Button", testID: "save", visible: false }] }),
  );
  const err = await rejection(page.getByTestId("save").click({ timeout: 250 }));
  bunExpect(err.message).toContain("element is not visible");
});

test("a zero-size widget is never clicked", async () => {
  const { page } = factory(() =>
    tree({ ref: 1, type: "Window", role: "window", children: [{ ref: 2, type: "Button", testID: "save", w: 0, h: 0 }] }),
  );
  const err = await rejection(page.getByTestId("save").click({ timeout: 250 }));
  bunExpect(err.message).toContain("element is zero size (0x0)");
});

test("a moving widget settles before it is clicked", async () => {
  // The button slides for the first four reads, then holds still.
  const client = new FakeClient((read) => {
    const t = tree({ ref: 1, type: "Window", role: "window", children: [{ ref: 2, type: "Button", testID: "save" }] });
    t.root.children[0]!.geometry!.x = read < 4 ? read : 4;
    return t;
  });
  await new Locator(client, [{ kind: "testid", value: "save" }]).click();
  bunExpect(client.calls.map((c) => c.method)).toEqual(["click"]);
  bunExpect(client.reads).toBeGreaterThan(4);
});

test("a stale ref sends the action back through resolve", async () => {
  const { app, page } = factory(() => twoButtons);
  app.fail("click", new AutomationRpcError(-32001, "not actionable"));
  await page.getByTestId("save").click();
  bunExpect(app.calls.filter((c) => c.method === "click").length).toBe(2);
});

test("an RPC the host does not have names the method", async () => {
  const { app, page } = factory(() => twoButtons);
  app.fail("focus", new AutomationRpcError(-32601, "method not found"));
  const err = await rejection(page.getByTestId("save").focus());
  bunExpect(err).toBeInstanceOf(LocatorError);
  bunExpect(err.message).toBe('host predates the "focus" RPC');
});

test("fill, check and setChecked ride setValue, and check no-ops when already set", async () => {
  const { app, page } = factory(() => twoButtons);
  await page.getByTestId("agree").check();
  await page.getByTestId("agree").uncheck();
  bunExpect(app.calls.filter((c) => c.method === "setValue")).toEqual([
    { method: "setValue", params: { ref: 4, value: true } },
  ]);
});

test("readers answer from one tree read", async () => {
  const { page } = factory(() => twoButtons);
  bunExpect(await page.getByRole("button").count()).toBe(2);
  bunExpect(await page.getByRole("button").allTextContents()).toEqual(["Save", "Save As"]);
  bunExpect(await page.getByTestId("save").textContent()).toBe("Save");
  bunExpect(await page.getByTestId("agree").inputValue()).toBe("false");
  bunExpect(await page.getByTestId("agree").isChecked()).toBe(false);
  bunExpect(await page.getByTestId("save").boundingBox()).toEqual({ x: 0, y: 0, width: 100, height: 20 });
  bunExpect(await page.getByTestId("nope").isVisible()).toBe(false);
  bunExpect(await page.getByTestId("save").getAttribute("role")).toBe("button");
});

test("expect polls until the tree catches up", async () => {
  const { page } = factory((read) =>
    tree({
      ref: 1,
      type: "Window",
      role: "window",
      children: [{ ref: 2, type: "Label", role: "label", testID: "count", text: read < 3 ? "2" : "3" }],
    }),
  );
  await expect(page.getByTestId("count")).toHaveText("3");
  await expect(page.getByTestId("count")).toContainText("3");
});

test("expect.not waits for the widget to go away", async () => {
  const { page } = factory((read) =>
    tree({
      ref: 1,
      type: "Window",
      role: "window",
      children: read < 3 ? [{ ref: 2, type: "Label", role: "label", testID: "toast", text: "Saved" }] : [],
    }),
  );
  await expect(page.getByTestId("toast")).not.toBeVisible();
  await expect(page.getByTestId("toast")).toHaveCount(0);
});

test("a failed matcher reports expected, received and the call log", async () => {
  const { page } = factory(() => twoButtons);
  const err = await rejection(expect(page.getByTestId("save")).toHaveText("Saved", { timeout: 200 }));
  bunExpect(err).toBeInstanceOf(TimeoutError);
  bunExpect(err.message).toContain("expect(testid=save).toHaveText: Timeout 200ms exceeded.");
  bunExpect(err.message).toContain('Expected: "Saved"');
  bunExpect(err.message).toContain('Received: "Save"');
  bunExpect(err.message).toContain("resolved 1 element");
});

test("expect over a plain value", () => {
  expect(3).toBe(3);
  expect({ a: [1] }).toEqual({ a: [1] });
  expect("ND_OK marker").toContain("ND_OK");
  expect(5).not.toBe(4);
  bunExpect(() => expect(4).toBe(5)).toThrow(/Expected: 5/);
});

test("Playwright key names become host chords", () => {
  bunExpect(toChord("Meta+A")).toBe("cmd+shift+a");
  bunExpect(toChord("Meta+a")).toBe("cmd+a");
  bunExpect(toChord("Control+Shift+n")).toBe("ctrl+shift+n");
  bunExpect(toChord("ArrowDown")).toBe("down");
  bunExpect(toChord("Escape")).toBe("escape");
  bunExpect(toChord("Enter")).toBe("enter");
  bunExpect(toChord("PageDown")).toBe("pagedown");
  bunExpect(toChord("F7")).toBe("f7");
  bunExpect(() => toChord("Shift+")).toThrow(/no key left/);
});

test("snapshot renders one line per identified node and collapses the rest", () => {
  const out = renderSnapshot(
    asNdNode(
      tree({
        ref: 1,
        type: "Window",
        role: "window",
        text: "Notes",
        children: [
          { ref: 5, type: "Box", children: [{ ref: 12, type: "Button", role: "button", testID: "save-btn", text: "Save" }] },
        ],
      }).root,
    ),
  );
  bunExpect(out.split("\n")).toEqual([
    '- window "Notes" [ref=e1] [enabled]',
    '  - button "Save" [ref=e12] [testid=save-btn] [enabled]',
  ]);
});

test("snapshot of a real capture keeps the interactive widgets", () => {
  const out = renderSnapshot(asFixtureRoot(), { interactiveOnly: true });
  bunExpect(out).toContain("[testid=volume-slider]");
  bunExpect(out).toContain("[testid=name-input]");
  bunExpect(out).not.toContain("[testid=volume-label]");
});

function asFixture(): GetTreeResult {
  return fixture as GetTreeResult;
}

function asFixtureRoot() {
  return asNdNode(asFixture().root);
}
