// Attach to a host somebody else launched (ND_AUTOMATION_SOCKET), with the
// same locator surface AppHandle carries.
//
// The acceptance gates start the host from bash and hand the socket path to a
// drive script, so those scripts never own a process. This is the half of
// AppHandle that does not need one: the tree, the host-side waits, the
// locators, and the keyboard and mouse.
import type {
  GetTreeResult,
  ScreenshotResult,
  SetWindowFrameParams,
  WaitCondition,
  WaitForResult,
  WindowInfo,
  WindowsResult,
} from "@nativedesktop/react/rpc";
import { AutomationClient } from "./socket.ts";
import { Locator, LocatorFactory, type LocatorClient, type RoleOptions, type TextOptions } from "./locator.ts";
import type { Keyboard, Mouse } from "./keyboard.ts";
import { renderWaitValue, type WaitOpts } from "./wait.ts";

const DEFAULT_ACTION_TIMEOUT_MS = 5_000;

export class AttachedApp implements LocatorClient {
  /** Deadline every locator action, reader and expect matcher inherits. */
  actionTimeout = DEFAULT_ACTION_TIMEOUT_MS;

  private readonly factory: LocatorFactory;

  private constructor(readonly rpc: AutomationClient) {
    this.factory = new LocatorFactory(this);
  }

  /** Connects to `path`, defaulting to ND_AUTOMATION_SOCKET. */
  static async connect(path = process.env.ND_AUTOMATION_SOCKET): Promise<AttachedApp> {
    return new AttachedApp(await AutomationClient.connect(path));
  }

  tree(window?: number): Promise<GetTreeResult> {
    return this.rpc.call("getTree", { window });
  }

  /** Untyped RPC entry point the Locator layer dispatches through, so one
   * call site serves every action method. */
  callRpc(method: string, params?: Record<string, unknown>): Promise<unknown> {
    return this.rpc.call(method as "click", params as never);
  }

  windows(): Promise<WindowsResult> {
    return this.rpc.call("windows");
  }

  /** The host's in-process render. Unlike AppHandle's, there is no
   * ScreenCaptureKit rung here: this process does not know the host's pid. */
  screenshot(path: string, opts: { window?: number } = {}): Promise<ScreenshotResult> {
    return this.rpc.call("screenshot", { path, window: opts.window });
  }

  isAlive(): boolean {
    return true;
  }

  /** Resizes a window in logical units, keeping its origin. The answer is
   * that window's updated WindowInfo, whose `geometry` is the same w/h
   * Geometry a node carries. */
  setWindowSize(width: number, height: number, opts: { window?: number } = {}): Promise<WindowInfo> {
    return this.rpc.call("setWindowFrame", { window: opts.window, width, height });
  }

  /** Moves and/or resizes a window; omitted components keep their current
   * value. GTK ignores x/y (client-side placement is not a Wayland
   * capability). */
  setWindowFrame(frame: SetWindowFrameParams): Promise<WindowInfo> {
    return this.rpc.call("setWindowFrame", frame);
  }

  // --- host-side waits -------------------------------------------------------

  waitFor(condition: WaitCondition, opts: WaitOpts = {}): Promise<WaitForResult> {
    return this.rpc.call("waitFor", { condition, timeoutMs: opts.timeoutMs, window: opts.window });
  }

  waitForText(text: string, opts?: WaitOpts): Promise<WaitForResult> {
    return this.waitFor({ textContains: text }, opts);
  }

  waitForPresent(testId: string, opts?: WaitOpts): Promise<WaitForResult> {
    return this.waitFor({ testId, state: "present" }, opts);
  }

  waitForGone(testId: string, opts?: WaitOpts): Promise<WaitForResult> {
    return this.waitFor({ testId, state: "gone" }, opts);
  }

  waitForValue(testId: string, value: string | number | boolean, opts: WaitOpts = {}): Promise<WaitForResult> {
    return this.waitFor({ testId, valueEquals: renderWaitValue(value) }, opts);
  }

  // --- locators --------------------------------------------------------------

  get keyboard(): Keyboard {
    return this.factory.keyboard;
  }

  get mouse(): Mouse {
    return this.factory.mouse;
  }

  locator(selector: string): Locator {
    return this.factory.locator(selector);
  }

  getByTestId(testId: string): Locator {
    return this.factory.getByTestId(testId);
  }

  getByRole(role: string, opts?: RoleOptions): Locator {
    return this.factory.getByRole(role, opts);
  }

  getByText(text: string | RegExp, opts?: TextOptions): Locator {
    return this.factory.getByText(text, opts);
  }

  getByLabel(text: string | RegExp, opts?: TextOptions): Locator {
    return this.factory.getByLabel(text, opts);
  }

  getByPlaceholder(text: string | RegExp, opts?: TextOptions): Locator {
    return this.factory.getByPlaceholder(text, opts);
  }

  /** A locator factory bound to one window, by index in `windows()` order or
   * by a substring of its title. */
  async window(titleOrIndex: string | number): Promise<LocatorFactory> {
    const { windows } = await this.windows();
    const info =
      typeof titleOrIndex === "number"
        ? windows[titleOrIndex]
        : windows.find((w) => (w.title ?? "").includes(titleOrIndex));
    if (!info) {
      const known = windows.map((w) => JSON.stringify(w.title ?? "")).join(", ") || "(none)";
      throw new Error(`window(${JSON.stringify(titleOrIndex)}): no such window. Open windows: ${known}`);
    }
    return new LocatorFactory(this, info.ref);
  }

  close(): Promise<void> {
    this.rpc.close();
    return Promise.resolve();
  }
}

/** Shorthand for AttachedApp.connect(). */
export function connectApp(path?: string): Promise<AttachedApp> {
  return AttachedApp.connect(path);
}
