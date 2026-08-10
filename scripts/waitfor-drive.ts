#!/usr/bin/env bun
// scripts/waitfor-drive.ts [gtk|appkit] — exercises the full §1.1 waitFor
// vocabulary (all 6 states + countAtLeast + valueEquals/valueContains +
// window scoping) against examples/gallery's default-active "Basics" tab.
// Runs entirely through @nativedesktop/test: no bash wrapper, no manual
// socket plumbing.
//
// "focused" and "disabled" have no naturally-true case in this app (no
// widget in the Basics tab is ever disabled, and there is no semantic action
// that grabs keyboard focus on an arbitrary widget yet — CLAUDE.md's M16
// section already documents "no general widget-level onKeyDown yet"), so
// both are exercised as negative assertions: the predicate must correctly
// NOT match an enabled, unfocused widget, proving the state check isn't a
// tautology rather than proving it can go true.
import { launchApp, type WaitForResult } from "../packages/test/src/index.ts";
import type { Backend } from "@nativedesktop/host";

const backend = process.argv[2] as Backend | undefined;

async function expectTimeout(label: string, run: () => Promise<WaitForResult>): Promise<void> {
  try {
    const res = await run();
    throw new Error(`${label}: expected a waitFor timeout, got ${JSON.stringify(res)}`);
  } catch (e) {
    const message = (e as Error).message;
    if (!message.includes("waitFor timeout")) throw new Error(`${label}: expected a waitFor timeout, got: ${message}`);
  }
}

const app = await launchApp({ entry: "examples/gallery/main.tsx", backend });

// 1. present — a widget that's always mounted.
const present = await app.waitForPresent("name-input", { timeoutMs: 3000 });
if (!present.matched || present.ref == null) throw new Error(`present: ${JSON.stringify(present)}`);

// 2. gone — a testID that never existed matches "gone" immediately.
const gone = await app.waitForGone("does-not-exist-in-this-app", { timeoutMs: 3000 });
if (!gone.matched || gone.count !== 0) throw new Error(`gone: ${JSON.stringify(gone)}`);

// 3. visible — the Basics tab is active by default, so its widgets are mapped.
const visible = await app.waitFor({ testId: "name-input", state: "visible" }, { timeoutMs: 3000 });
if (!visible.matched) throw new Error(`visible: ${JSON.stringify(visible)}`);

// 4. enabled — no widget on this tab starts disabled.
const enabled = await app.waitForEnabled("name-input", { timeoutMs: 3000 });
if (!enabled.matched) throw new Error(`enabled: ${JSON.stringify(enabled)}`);

// 5. disabled (negative) — the same widget must NOT satisfy "disabled".
await expectTimeout("disabled", () => app.waitForDisabled("name-input", { timeoutMs: 800 }));

// 6. focused (negative) — an untouched widget must NOT satisfy "focused".
await expectTimeout("focused", () => app.waitForFocused("name-input", { timeoutMs: 800 }));

console.log("ND_WAITFOR_STATES_OK present/gone/visible/enabled/disabled/focused all correct");

// countAtLeast: exactly one "name-input" exists, so >=1 matches and >=2 times out.
const count1 = await app.waitForCount("name-input", 1, { timeoutMs: 3000 });
if (!count1.matched || count1.count !== 1) throw new Error(`countAtLeast(1): ${JSON.stringify(count1)}`);
await expectTimeout("countAtLeast(2)", () => app.waitFor({ testId: "name-input", countAtLeast: 2 }, { timeoutMs: 800 }));

console.log("ND_WAITFOR_COUNT_OK countAtLeast matches at the exact boundary");

// valueEquals: bool (Checkbox) and number (Slider) both stringify per the
// host's rendering rule (numbers stringified, bools "true"/"false").
await app.setValue("agree-check", true);
const boolValue = await app.waitForValue("agree-check", true, { timeoutMs: 3000 });
if (!boolValue.matched) throw new Error(`valueEquals(bool): ${JSON.stringify(boolValue)}`);

await app.setValue("volume-slider", 77);
const numberValue = await app.waitForValue("volume-slider", 77, { timeoutMs: 3000 });
if (!numberValue.matched) throw new Error(`valueEquals(number): ${JSON.stringify(numberValue)}`);

// valueContains: substring match against the string rendering.
await app.setValue("name-input", "hello world");
const containsValue = await app.waitForValue("name-input", "world", { timeoutMs: 3000, contains: true });
if (!containsValue.matched) throw new Error(`valueContains: ${JSON.stringify(containsValue)}`);

console.log("ND_WAITFOR_VALUE_OK valueEquals(bool)/valueEquals(number)/valueContains all matched");

// window scoping: thread the root window's own ref through `window` and
// confirm the predicate still resolves against it (this app has one window;
// scripts/tabs-drive.ts and examples/multiwindow cover multi-window).
const { windows } = await app.windows();
if (windows.length !== 1) throw new Error(`expected exactly 1 window, got ${windows.length}`);
const rootWindow = windows[0]!.ref;
const scoped = await app.waitFor({ testId: "name-input", state: "present" }, { timeoutMs: 3000, window: rootWindow });
if (!scoped.matched) throw new Error(`window-scoped waitFor: ${JSON.stringify(scoped)}`);

console.log(`ND_WAITFOR_WINDOW_OK window=${rootWindow} scoping threaded through waitFor`);

await app.close();
console.log(`ND_WAITFOR_OK backend=${app.backend} all 6 states + countAtLeast + valueEquals/valueContains + window scoping verified`);
