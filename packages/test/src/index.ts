// @nativedesktop/test — §1.4: launchApp/AppHandle over the automation socket.
// Reuses packages/mcp/src/socket.ts's AutomationClient verbatim (wrapped for
// per-call timeouts in client.ts) rather than forking the framing code.
export { launchApp, AppHandle, killAll } from "./launch.ts";
export type { LaunchOptions } from "./launch.ts";

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

// Tree types a caller needs to type find()/tree() results, re-exported so
// nothing outside this package has to reach into packages/react/src/generated.
export type { GetTreeResult, JsonNode } from "../../react/src/generated/rpc.ts";
