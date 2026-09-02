---
title: Locators
description: "Playwright semantics over the automation socket: lazy locators, actionability, strict mode, and polling expect matchers."
---

A locator names a widget; it does not hold one. Every action and every reader re-resolves from a
fresh `getTree`, so a locator kept across a re-render still points at the right thing, and a ref
that went stale between resolve and dispatch is retried rather than thrown.

```ts
import { expect, launchApp } from "@nativedesktop/test";

const app = await launchApp({ entry: "examples/counter/main.tsx" });

await app.getByTestId("increment-button").click();
await expect(app.getByTestId("clicks-label")).toContainText("Clicks: 1");

await app.getByRole("textbox").fill("Grocery run");
await app.getByRole("button", { name: "Save" }).click();
await expect(app.getByTestId("toast")).not.toBeVisible();

await app.close();
```

Attaching to a host somebody else launched (the shape every `scripts/*-drive.ts` uses, since the
acceptance gates own the process) gives the same surface:

```ts
import { connectApp, expect } from "@nativedesktop/test";

const app = await connectApp(); // reads ND_AUTOMATION_SOCKET
await expect(app.getByTestId("volume-slider")).toHaveValue("20");
```

## Finding widgets

| Factory | Matches on |
|---|---|
| `getByTestId(id)` | the node's `testID`, exactly |
| `getByRole(role, {name?, exact?, checked?, disabled?})` | the schema-declared automation role, plus its accessible name |
| `getByText(text, {exact?})` | the node's own `text` |
| `getByLabel(text, {exact?})` | the accessible name (`label`, else `text`, else a string `value`) |
| `getByPlaceholder(text, {exact?})` | the node's `placeholder` |
| `locator(selector)` | a selector string (below) |

A string match is a case-insensitive substring after whitespace normalisation. Pass `exact: true`,
or a `RegExp`, for anything stricter.

Locators chain, and the chain descends: `app.getByTestId("sidebar").getByRole("button")` finds
buttons inside the sidebar. `filter({hasText, has, hasNot})`, `and()`, and `first()` / `last()` /
`nth(i)` narrow the current match set without descending.

### Selector strings

`locator()` takes the same parts as a string, joined by `>>`:

```ts
app.locator('role=button[name="Save"] >> nth=1');
app.locator("type=Table >> has-text=Ada >> first");
app.locator("testid=list >> role=button >> last");
```

Engines: `testid=`, `role=x[name="..."|name=/re/][exact][checked][disabled]`, `text=`, `label=`,
`placeholder=`, `type=` (the ND widget type, e.g. `type=Button`). Refinements: `nth=i` (negative
counts from the end), `first`, `last`, `has-text=`, `has=(...)`, `has-not=(...)`, `and=(...)`. A
quoted value means an exact match; `/re/flags` is a regular expression.

## Actions and readers

Actions: `click`, `dblclick`, `rightClick`, `hover`, `fill`, `type`, `pressSequentially`, `press`,
`check`, `uncheck`, `setChecked`, `selectOption`, `focus`, `blur`, `scrollIntoViewIfNeeded`,
`dragTo`, `screenshot`.

Readers: `boundingBox`, `textContent`, `inputValue`, `getAttribute`, `isVisible`, `isEnabled`,
`isChecked`, `isFocused`, `count`, `all`, `allTextContents`, `node`, `ref`, and
`waitFor({state: "attached" | "visible" | "hidden" | "detached"})`.

`fill` replaces a value the way a user finishing an edit would; `type` appends, matching the `type`
RPC. `count()` and `isVisible()` take one tree read and never wait; the other readers wait for the
node to exist.

`app.keyboard` and `app.mouse` cover what no widget owns: `keyboard.press("Meta+A")` in Playwright
key names, `keyboard.type("hello")`, `mouse.click(x, y)` / `dblclick` / `dragTo` in
logical-window-topleft coordinates. Those ride the input-synthesis RPCs, so they answer `-32003` on
GTK.

## Actionability

Before a single-target action a locator resolves every 100ms until the deadline (`app.actionTimeout`,
5000ms by default, or a per-call `{timeout}`), then requires the one match to be:

1. the only match. Two matches is a `StrictModeError`, thrown at once rather than waited out, because
   a second element is a selector bug and not a timing one.
2. visible, enabled, and occupying real pixels.
3. still in the same rectangle on the next read, so a widget mid-animation is never clicked.

Readers and `screenshot` skip the stable-frame wait; they only need the node to exist.

A timeout says what it waited for and what was nearby:

```
locator.click: Timeout 5000ms exceeded.
Call log:
  - waiting for testid=save-btn
  - resolved 0 elements
Nearest candidates in window "Notes" (3):
  Button  role=button  text="Save As"  testID=save-as  visible enabled  (12,340 88x28)
```

## expect

`expect(locator)` polls until the matcher passes or the deadline expires; `.not` inverts the
predicate and keeps polling, so `.not.toBeVisible()` waits for a widget to go away.

`toBeVisible`, `toBeHidden`, `toBeAttached`, `toBeEnabled`, `toBeDisabled`, `toBeChecked`,
`toBeFocused`, `toHaveText`, `toContainText`, `toHaveValue`, `toHaveCount`, `toHaveAttribute`.

`expect(value)` on anything that is not a locator is a plain, non-polling assertion (`toBe`,
`toEqual`, `toContain`, `toMatch`, `toBeGreaterThan`, and the rest), so a drive script needs only
one assertion vocabulary.

## What locators do not replace

`waitFor` and its sugar (`app.waitForText`, `waitForPresent`, `waitForGone`, `waitForValue`, plus the
page predicates `urlContains` / `pageTitleContains` / `pageTextContains`) run **host-side** on the
retained tree at a 50ms tick, with no `getTree` round trip. Where a script already waits that way,
leave it: it is cheaper than any client-side poll, and it is the only path to the page predicates.

Reach past a locator with `app.rpc.call(...)` when you need a sub-region of a widget (a table row
band, a tab strip, a row's trailing button), a widget-level field the tree models but locators do not
(`itemCount`, a SourceTree's `rows`), or a webview RPC. `locator.ref()` and `locator.node()` hand you
the resolved ref and node for exactly that.

## Host methods a locator needs

`focus`, `scrollIntoView`, `snapshotNode` and `setWindowFrame` are separate RPCs. Against a host that
predates one, the call fails with `host predates the "<method>" RPC` rather than a bare `-32601`. The
optional node fields (`checked`, `label`, `placeholder`, `options`, `selected`, `expanded`) are null
on a node the field does not apply to and on a host that predates them, and each reader falls back to
`text` and `value` there, so a selector behaves the same either way.

`app.setWindowSize(w, h)` (and `app.setWindowFrame({x, y, width, height})` for a move) answers the
window's updated `WindowInfo`, whose `geometry` is the same `{x, y, w, h}` a node carries.
`locator.screenshot()` renders that one node through `snapshotNode`; pass a path or let the host
write beside the automation socket and tell you where.

## Acceptance

`scripts/locator-drive.ts` against `examples/locators` is the gate for this surface: `focus()` plus
`toBeFocused`, `press("Meta+a")`, `scrollIntoViewIfNeeded()` on a clipped row, a node-sized
`screenshot()`, `setWindowSize` with the root's `boundingBox()` following, `isChecked()` across
`check()`/`uncheck()`, and `selectOption("Downloads")` by label. It prints `ND_LOCATOR_OK` and runs
as the third leg of `scripts/mac/mac-gestures.sh`.
