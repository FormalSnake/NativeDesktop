// @nativedesktop/test — §1.4: launchApp/AppHandle over the automation socket.
// socket.ts's AutomationClient is the repo's ONE copy of the wire framing;
// packages/mcp and the drive scripts import it from here rather than forking.
export { AutomationClient, AutomationRpcError } from "./socket.ts";
export { launchApp, AppHandle, killAll } from "./launch.ts";
export type { LaunchOptions, NdWindowInfo } from "./launch.ts";

export { TimedClient } from "./client.ts";

export { resolveTarget, findNode, findAllNodes, findMatchingNode } from "./query.ts";
export type { Target } from "./query.ts";

export { poll, renderWaitValue } from "./wait.ts";
export type { WaitOpts } from "./wait.ts";

export { takeScreenshot } from "./screenshot.ts";
export type { ScreenshotOptions } from "./screenshot.ts";

export { dialogScriptEnv } from "./dialogs.ts";
export type { DialogScript } from "./dialogs.ts";

export { pngSize } from "./png.ts";
export type { PngSize } from "./png.ts";

// Locators: Playwright semantics over getTree plus the action RPCs. A Locator
// is lazy and re-resolves on every use, so a drive script never holds a ref
// across a re-render.
export { AttachedApp, connectApp } from "./attach.ts";
export { Locator, LocatorFactory, callHost } from "./locator.ts";
export type { ActionOptions, BoundingBox, LocatorClient, RoleOptions, TextOptions, WaitForState } from "./locator.ts";
export { expect, LocatorAssertions, ValueAssertions } from "./expect.ts";
export type { ExpectOptions } from "./expect.ts";
export { Keyboard, Mouse, toChord } from "./keyboard.ts";
export { LocatorError, StrictModeError, TimeoutError, describeNode, rankCandidates } from "./errors.ts";
export { formatSelector, parseSelector } from "./selectors.ts";
export type { SelectorPart, TextSpec } from "./selectors.ts";
export {
  allNodes,
  asNdNode,
  hasRealSize,
  matchPart,
  nodeChecked,
  nodeName,
  nodeText,
  renderValue,
  selectNodes,
  subtreeText,
} from "./matcher.ts";
export type { NdNode } from "./matcher.ts";
export { renderSnapshot } from "./snapshot.ts";
export type { SnapshotOptions } from "./snapshot.ts";

// Tree types a caller needs to type find()/tree() results, re-exported so
// nothing outside this package has to import @nativedesktop/react/rpc itself.
export type { GetTreeResult, JsonNode } from "@nativedesktop/react/rpc";
