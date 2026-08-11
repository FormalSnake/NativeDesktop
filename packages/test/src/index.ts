// @nativedesktop/test — §1.4: launchApp/AppHandle over the automation socket.
// socket.ts's AutomationClient is the repo's ONE copy of the wire framing;
// packages/mcp and the drive scripts import it from here rather than forking.
export { AutomationClient } from "./socket.ts";
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
// nothing outside this package has to import @nativedesktop/react/rpc itself.
export type { GetTreeResult, JsonNode } from "@nativedesktop/react/rpc";
