---
title: Test Harness
description: "@nativedesktop/test: launchApp and AppHandle. Launch a host, connect, query, act, wait on the tree, and screenshot from a Bun test or drive script."
---

`@nativedesktop/test` wraps the [automation socket](/automation-testing/automation-socket/) into a
`launchApp`/`AppHandle` API: spawn a host binary, wait for it to be ready, connect, and drive it
with the same RPC surface, without hand-rolling process spawn, marker parsing, socket connect, or a
`find`/`mustFind` tree walker in every script. Wire framing comes from its own `src/socket.ts`
`AutomationClient`, which `packages/mcp` imports rather than keeping a second implementation.

```ts
import { launchApp } from "@nativedesktop/test";

const app = await launchApp({ entry: "examples/notes/main.tsx", backend: "gtk" });

await app.click("new-note-button");
await app.waitForText("Untitled note");
await app.setValue("title-input", "Grocery run");

const shot = await app.screenshot("/tmp/notes.png");
console.log(`${shot.width}x${shot.height}`);

await app.close();
```

Anything targeting a widget reads better through a
[locator](/automation-testing/locators/), which re-resolves on every use and polls until the widget
is actionable. The members below are the process, the raw RPC, and the host-side waits underneath
it.

## `launchApp(options)`

```ts
const app = await launchApp({
  entry: "src/main.tsx",            // -> ND_SCRIPT
  cwd?: string,                     // default process.cwd()
  backend?: "gtk" | "appkit",       // default @nativedesktop/host's resolveBackend()
  hostBinary?: string,              // pre-resolved binary path, bypasses resolveHostBinary()
  env?: Record<string, string | undefined>,
  dev?: boolean,                    // ND_DEV=1, default false
  acl?: Record<string, string[]>,   // -> ND_ACL_GRANTS JSON
  dialogScript?: DialogScript,      // -> ND_AUTOMATION_DIALOG_SCRIPT (see below)
  readyMarkers?: string[],          // default ["ND_AUTOMATION_LISTENING", "ND_COMMIT_APPLIED"]
  readyTimeoutMs?: number,          // default 20_000
  rpcTimeoutMs?: number,            // default 8_000
  retries?: number,                 // relaunch attempts on ready failure/early exit, default 2
  logPath?: string,
  onStderr?: (line: string) => void,
});
```

`hostBinary` is for callers `@nativedesktop/host`'s own resolution cannot place: a consumer outside
a NativeDesktop checkout (installed as a `file:` or `link:` dep, so the source-checkout fallback
misses it too), or the GTK-on-macOS dev path, where no prebuilt ships. Resolve `nd-hello` from a
sibling checkout yourself and pass its path.

`entry` resolves relative to `cwd`. The GTK host, including GTK-via-Quartz on macOS, fails to start
when `XDG_RUNTIME_DIR` is unset or points at a directory that does not exist. `launchApp` creates a
fresh one unless the environment already provides a valid one, so callers never have to remember
`export XDG_RUNTIME_DIR="$(mktemp -d)"`.

If the ready markers never appear, or the process exits first, `launchApp` kills it and retries up
to `retries` more times before rejecting with all `retries + 1` failures and the last 40 lines of
stderr.

## `AppHandle`

| Member | Notes |
|---|---|
| `pid`, `logPath`, `socketPath`, `backend` | identity of the current live process |
| `rpc` | the wrapped `AutomationClient`. Every call races a `rpcTimeoutMs` timeout and, on timeout, rejects with `` `${method} timed out after Nms` `` plus the last 40 stderr lines |
| `stderr()` / `stderrTail(n = 40)` | the full captured stderr, or its last `n` lines |
| `tree(window?)` | raw `getTree` result |
| `find(testId, {window?})` / `findAll(testId, {window?})` / `mustFind(testId, {window?})` / `findMatching(pred, {window?})` | tree-walk helpers over the current snapshot; `mustFind` throws when nothing matches |
| `click(t)`, `setValue(t, v)`, `type(t, s)`, `scroll(t, {dx?, dy?})`, `hover(t)`, `doubleClick(t)`, `rightClick(t)` | single-RPC, actionability-checked, host-side `testId` resolution, no client-side retry loop |
| `keys(spec, {window?})`, `drag(opts)` | input synthesis; macOS only, `-32003` on GTK |
| `waitFor(condition, {timeoutMs?, window?})` plus the sugar below | thin pass-throughs to the `waitFor` RPC. The host polls, not this client |
| `waitForMarker(marker, timeoutMs?)` | polls captured stderr for a substring (dialog-script exhaustion, crash markers, anything else the host logs) |
| `windows()` / `waitForWindows(count, timeoutMs?)` | the `windows` RPC / poll it until the count matches |
| `screenshot(path, opts?)` | see below |
| `locator(selector)`, `getByTestId`, `getByRole`, `getByText`, `getByLabel`, `getByPlaceholder`, `keyboard`, `mouse`, `window(titleOrIndex)`, `actionTimeout` | the [locator surface](/automation-testing/locators/) |
| `setWindowSize(w, h, {window?})` | `setWindowFrame` RPC, keeping the window's origin |
| `isAlive()` | whether the host process is still running |
| `restart()` | tears down the current process and relaunches with the same options; app state resets to a fresh launch |
| `close()` / `kill()` | graceful (`SIGTERM`, falls back to `SIGKILL` after 3s) / immediate |
| `[Symbol.asyncDispose]` | `await using app = await launchApp(...)` closes it automatically |

Every action method takes a target: `t: string | number | {testId?, ref?, window?, action?}`. A bare
string is a `testId`, a bare number is a `ref`, and an object descriptor passes through. `action`
applies to `click` only: `app.click({ testId: "row-testid", action: "action-id" })` invokes a
SourceTree row's trailing action semantically (see
[SourceTree row actions](/automation-testing/automation-socket/#sourcetree-row-actions)). Exactly
one of `ref` or `testId` must resolve; the host validates it and answers `invalidParams` otherwise.

### waitFor sugar

Each of these is a single `waitFor` RPC call with no client-side polling:

| Method | Condition |
|---|---|
| `waitForText(text, opts?)` | `{textContains: text}` |
| `waitForPresent(testId, opts?)` | `{testId, state: "present"}` |
| `waitForGone(testId, opts?)` | `{testId, state: "gone"}` |
| `waitForEnabled(testId, opts?)` | `{testId, state: "enabled"}` |
| `waitForDisabled(testId, opts?)` | `{testId, state: "disabled"}` |
| `waitForFocused(testId, opts?)` | `{testId, state: "focused"}` |
| `waitForCount(testId, count, opts?)` | `{testId, countAtLeast: count}` |
| `waitForValue(testId, value, opts?)` | `{testId, valueEquals: render(value)}`, or `valueContains` when `opts.contains` is `true` |
| `waitForUrl(testId, substring, opts?)` | `{testId, urlContains: substring}` |
| `waitForPageTitle(testId, substring, opts?)` | `{testId, pageTitleContains: substring}` |
| `waitForPageText(testId, substring, opts?)` | `{testId, pageTextContains: substring}` |

`opts` is `{timeoutMs?, window?}` (`waitForValue` also takes `contains?: boolean`). See
[waitFor conditions](/automation-testing/automation-socket/#waitfor-conditions) for exactly what
each `state` checks and how values are rendered to a string.

A `timeoutMs` larger than the client's own `rpcTimeoutMs` is honoured: the harness raises the
client deadline for any call that declares one, so a 30s `waitFor` is not cut short at the 8s
default.

### Browser helpers

The last three above name a `<webview>` testID. `waitForPageText` injects
`document.body.innerText` into the page — see
[page predicates](/automation-testing/automation-socket/#page-predicates).

| Method | Does |
|---|---|
| `webviewInfo(target, opts?)` | `{ref, url, title, loading, canGoBack, canGoForward}`, read off the engine |
| `evalInPage(target, code, opts?)` | evaluates in the page; `opts.world` picks a named isolated world. A thrown exception is `{ok: false, error}`, not a rejection |
| `openAndAwaitLoad(testId, url, opts?)` | navigates the view and waits for the page to commit that URL |
| `screenshotPage(path, opts?)` | `screenshot` with a byte floor that rejects a blank frame |

```ts
await app.openAndAwaitLoad("tab-webview", "https://example.com/");
const { value } = await app.evalInPage({ testId: "tab-webview" }, "document.title");
await app.waitForPageText("tab-webview", "Example Domain");
```

`openAndAwaitLoad` drives the engine (`location.href = …`) rather than the widget's `url` prop,
so it works without the app wiring anything up; it then matches the URL minus its scheme, because
engines normalise trailing slashes, percent-encoding and http/https upgrades.

### `screenshot(path, opts?)`

```ts
interface ScreenshotOptions {
  window?: number;
  retries?: number;     // default 5
  minHeight?: number;   // throw if the PNG is shorter than this
  minBytes?: number;    // throw if the PNG file is smaller than this
  via?: "rpc" | "ndshot"; // "ndshot" shells out to tools/ndshot, macOS/AppKit only
}
```

Retries with a 150ms backoff (a screenshot right after a mutation or animation can race frame
invalidation and answer a transient error or a stale frame). Every call is floor-checked for
non-zero dimensions regardless of `minHeight`/`minBytes`. `pngSize(path)` (also exported) parses a
PNG's width/height from its IHDR chunk, independent of the harness.

`via: "ndshot"` runs `tools/ndshot capture --out <path> --pid <app.pid>` instead of the in-process
`screenshot` RPC. That is the workaround for macOS 26's offscreen-render blanking of `TextInput` and
`TextArea`, documented in
[ndshot](/automation-testing/automation-socket/#screenshots-on-macos-ndshot). It requires
`tools/ndshot/build.sh` to have run at least once.

### Dialog scripts

`dialogScript` takes the same shape `ND_AUTOMATION_DIALOG_SCRIPT` parses. See
[Scripted native dialogs](/automation-testing/automation-socket/#scripted-native-dialogs) for the
per-method entry shapes and the exhaustion contract:

```ts
import type { DialogScript } from "@nativedesktop/test";

const dialogScript: DialogScript = {
  "dialog.openFile": [["/tmp/a.txt"]],
  "window.showAlert": [{ buttonId: "delete" }],
};
const app = await launchApp({ entry: "examples/dialogs/main.tsx", dialogScript });
```

### Lifecycle and cleanup

`killAll()`, also exported at module scope, kills every host process any `launchApp` call in the
current process has spawned. It is wired to `process.on("exit"/"SIGINT"/"SIGTERM")`, so a thrown
assertion partway through a script never leaves an orphaned window behind. Use `await app.close()`
when a test finishes normally, and `killAll()` in a top-level `finally` or a test runner's global
teardown instead of tracking handles yourself.

## Also exported

- `connectApp(path?)` / `AttachedApp`: the same locator surface, plus `tree`, the `waitFor` sugar,
  `windows()` and `screenshot()`, over a host somebody else launched (`ND_AUTOMATION_SOCKET`). This
  is what every acceptance gate's drive script uses, since the gate's bash owns the process.
- `expect(locatorOrValue)`: polling locator matchers, and plain assertions for everything else. See
  [Locators](/automation-testing/locators/).
- `pngSize(path)`: PNG dimensions from the IHDR chunk. Used by `screenshot()`, useful standalone.
- `poll(fn, pred, {timeoutMs?, intervalMs?})`: generic poll-until-predicate, for the rare condition
  `waitFor`'s vocabulary does not cover, such as window count settling or a `SourceList`'s `rows`
  array reordering after a click.
- `resolveTarget(t)`, `findNode`, `findAllNodes`, `findMatchingNode`: the target-normalization and
  tree-walk primitives `AppHandle` is built on, for scoped subtree searches like
  `findNode(paneNode, "some-child-testid")` rather than a whole-tree `find`.
- `JsonNode`, `GetTreeResult`: re-exported from `packages/react/src/generated/rpc.ts` so a caller
  never has to reach into the generated tree outside this package.
