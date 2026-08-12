// §1.4's launchApp/AppHandle: spawns a host binary with NATIVE_AUTOMATION=1,
// waits for the ready markers on stderr, and connects socket.ts's
// AutomationClient (wrapped in client.ts's TimedClient) over the parsed
// socket path. One AppHandle owns exactly one host process at a time;
// restart() tears it down and relaunches in place.
import { existsSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Backend } from "@nativedesktop/host";
import { resolveBackend, resolveHostBinary } from "@nativedesktop/host";
import { AutomationClient } from "./socket.ts";
import type {
  DragParams,
  DragResult,
  ClickResult,
  GetTreeResult,
  JsonNode,
  KeysResult,
  ScreenshotResult,
  ScrollResult,
  SetValueResult,
  TypeResult,
  WaitCondition,
  WaitForResult,
  WindowsResult,
} from "@nativedesktop/react/rpc";
import { TimedClient } from "./client.ts";
import { type DialogScript, dialogScriptEnv } from "./dialogs.ts";
import { findAllNodes, findMatchingNode, findNode, resolveTarget, type Target } from "./query.ts";
import { registerProcess, unregisterProcess } from "./registry.ts";
import { type ScreenshotOptions, takeScreenshot } from "./screenshot.ts";
import { poll, renderWaitValue, type WaitOpts } from "./wait.ts";

export interface LaunchOptions {
  /** Entry file, e.g. "src/main.tsx" -> ND_SCRIPT. */
  entry: string;
  cwd?: string;
  backend?: Backend;
  /** Pre-resolved host binary path, bypassing @nativedesktop/host's own
   * resolution. For callers @nativedesktop/host can't place on its own: a
   * consumer outside a NativeDesktop checkout (installed as a `file:`/`link:`
   * dep, so the source-checkout fallback can't find it either), or the
   * gtk-on-macOS dev path (no prebuilt ships there by design). */
  hostBinary?: string;
  env?: Record<string, string | undefined>;
  /** ND_DEV=1. Default false. */
  dev?: boolean;
  /** -> ND_ACL_GRANTS JSON. */
  acl?: Record<string, string[]>;
  /** -> ND_AUTOMATION_DIALOG_SCRIPT (§1.5). */
  dialogScript?: DialogScript;
  readyMarkers?: string[];
  readyTimeoutMs?: number;
  rpcTimeoutMs?: number;
  /** Relaunch attempts on ready failure / early exit, on top of the first try. Default 2. */
  retries?: number;
  logPath?: string;
  onStderr?: (line: string) => void;
}

const DEFAULT_READY_MARKERS = ["ND_AUTOMATION_LISTENING", "ND_COMMIT_APPLIED"];
const DEFAULT_READY_TIMEOUT_MS = 20_000;
const DEFAULT_RPC_TIMEOUT_MS = 8_000;
const DEFAULT_RETRIES = 2;
const MAX_STDERR_LINES = 5000;
const STDERR_DRAIN_GRACE_MS = 1000;
const SOCKET_PATH_RE = /ND_AUTOMATION_LISTENING path=(\S+)/;

function buildEnv(opts: LaunchOptions): Record<string, string> {
  const merged: Record<string, string | undefined> = {
    ...process.env,
    NATIVE_AUTOMATION: "1",
    ND_SCRIPT: opts.entry,
  };
  if (opts.dev) merged.ND_DEV = "1";
  if (opts.acl) merged.ND_ACL_GRANTS = JSON.stringify(opts.acl);
  if (opts.dialogScript) merged.ND_AUTOMATION_DIALOG_SCRIPT = dialogScriptEnv(opts.dialogScript);
  Object.assign(merged, opts.env);
  const env: Record<string, string> = {};
  for (const [key, value] of Object.entries(merged)) {
    if (value !== undefined) env[key] = value;
  }
  // The GTK host (including GTK-via-Quartz on macOS) hard-fails during
  // startup (ND_RUNTIME_ERROR nd_start_runtime failed) when XDG_RUNTIME_DIR
  // is unset or points at a directory that doesn't exist. Every hand-written
  // bash driver works around this with `mktemp -d`; do it here once so no
  // caller has to remember the quirk.
  if (!env.XDG_RUNTIME_DIR || !existsSync(env.XDG_RUNTIME_DIR)) {
    env.XDG_RUNTIME_DIR = mkdtempSync(join(tmpdir(), "nd-test-xdg-"));
  }
  return env;
}

interface StderrPump {
  /** Resolves once the stream hits EOF, or once `cancel()` forces it to stop. */
  done: Promise<void>;
  /** Stops the pump without waiting for EOF -- the host's own child (spawned
   * with `.stderr = .inherit`, see src/runtime.zig) can outlive the host and
   * keep the pipe's write end open, so EOF is not guaranteed. */
  cancel: () => void;
}

/** Splits a byte stream into lines, decoding across chunk boundaries. */
function pumpStderr(
  stream: ReadableStream<Uint8Array>,
  onLine: (line: string) => void,
  onChunk?: (chunk: Uint8Array) => void,
): StderrPump {
  const reader = stream.getReader();
  const decoder = new TextDecoder();
  let buf = "";
  const done = (async () => {
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        onChunk?.(value);
        buf += decoder.decode(value, { stream: true });
        let idx: number;
        while ((idx = buf.indexOf("\n")) >= 0) {
          onLine(buf.slice(0, idx));
          buf = buf.slice(idx + 1);
        }
      }
    } catch {
      // reader.cancel() below rejects the in-flight read; nothing left to drain.
    }
    if (buf.length) onLine(buf);
  })();
  return { done, cancel: () => void reader.cancel().catch(() => {}) };
}

interface ResolvedConfig {
  binary: string;
  cwd: string;
  env: Record<string, string>;
  readyMarkers: string[];
  readyTimeoutMs: number;
  rpcTimeoutMs: number;
  retries: number;
  backend: Backend;
  logPath?: string;
  onStderr?: (line: string) => void;
}

interface LineSink {
  write(chunk: Uint8Array): number;
  end(): void;
}

interface LiveHost {
  proc: import("bun").Subprocess<"ignore", "ignore", "pipe">;
  pid: number;
  stderrLines: string[];
  seenMarkers: Set<string>;
  socketPath: string | undefined;
  exited: boolean;
  exitCode: number | null;
  pumpDone: Promise<void>;
  cancelStderr: () => void;
  exitedPromise: Promise<number>;
}

export class AppHandle {
  private live?: LiveHost;
  private client?: TimedClient;

  private constructor(private readonly config: ResolvedConfig) {}

  static async launch(config: ResolvedConfig): Promise<AppHandle> {
    const handle = new AppHandle(config);
    await handle.launchWithRetries();
    return handle;
  }

  private async launchWithRetries(): Promise<void> {
    let lastErr: Error | undefined;
    for (let attempt = 0; attempt <= this.config.retries; attempt++) {
      try {
        await this.spawnOnce();
        await this.connectRpc();
        return;
      } catch (e) {
        lastErr = e as Error;
      }
    }
    throw new Error(`launchApp: failed after ${this.config.retries + 1} attempt(s): ${lastErr?.message}`);
  }

  private async spawnOnce(): Promise<void> {
    const proc = Bun.spawn([this.config.binary], {
      cwd: this.config.cwd,
      env: this.config.env,
      stdin: "ignore",
      stdout: "ignore",
      stderr: "pipe",
    });
    registerProcess(proc);

    const logSink: LineSink | undefined = this.config.logPath ? Bun.file(this.config.logPath).writer() : undefined;
    const live: LiveHost = {
      proc,
      pid: proc.pid,
      stderrLines: [],
      seenMarkers: new Set(),
      socketPath: undefined,
      exited: false,
      exitCode: null,
      pumpDone: Promise.resolve(),
      cancelStderr: () => {},
      exitedPromise: Promise.resolve(0),
    };
    this.live = live;

    const onLine = (line: string): void => {
      live.stderrLines.push(line);
      if (live.stderrLines.length > MAX_STDERR_LINES) live.stderrLines.shift();
      for (const marker of this.config.readyMarkers) {
        if (line.includes(marker)) live.seenMarkers.add(marker);
      }
      const match = SOCKET_PATH_RE.exec(line);
      if (match?.[1]) live.socketPath = match[1];
      this.config.onStderr?.(line);
    };

    const pump = pumpStderr(proc.stderr, onLine, logSink ? (chunk) => void logSink.write(chunk) : undefined);
    live.pumpDone = pump.done.finally(() => logSink?.end());
    live.cancelStderr = pump.cancel;
    live.exitedPromise = proc.exited.then((code) => {
      live.exited = true;
      live.exitCode = code;
      unregisterProcess(proc);
      return code;
    });

    try {
      await this.waitUntilReady(live);
    } catch (e) {
      if (!live.exited) proc.kill("SIGKILL");
      await live.exitedPromise.catch(() => {});
      throw e;
    }
  }

  private async waitUntilReady(live: LiveHost): Promise<void> {
    const deadline = Date.now() + this.config.readyTimeoutMs;
    for (;;) {
      if (this.config.readyMarkers.every((m) => live.seenMarkers.has(m))) return;
      if (live.exited) {
        throw new Error(
          `host exited before ready markers were seen (code ${live.exitCode})\n` +
            `--- last 40 stderr lines ---\n${tail(live.stderrLines, 40)}`,
        );
      }
      if (Date.now() >= deadline) {
        throw new Error(
          `ready markers not seen within ${this.config.readyTimeoutMs}ms: ${this.config.readyMarkers.join(", ")}\n` +
            `--- last 40 stderr lines ---\n${tail(live.stderrLines, 40)}`,
        );
      }
      await new Promise((r) => setTimeout(r, 50));
    }
  }

  private async connectRpc(): Promise<void> {
    const live = this.live;
    if (!live) throw new Error("connectRpc: no live host");
    if (!live.socketPath) {
      throw new Error(
        `no automation socket path parsed from stderr (ND_AUTOMATION_LISTENING line missing)\n` +
          `--- last 40 stderr lines ---\n${tail(live.stderrLines, 40)}`,
      );
    }
    const inner = await AutomationClient.connect(live.socketPath);
    this.client = new TimedClient(inner, this.config.rpcTimeoutMs, (n) => this.stderrTail(n));
  }

  private async teardown(): Promise<void> {
    this.client?.close();
    this.client = undefined;
    const live = this.live;
    if (!live) return;
    if (!live.exited) {
      live.proc.kill("SIGTERM");
      const exited = await Promise.race([
        live.exitedPromise.then(() => true),
        new Promise<boolean>((r) => setTimeout(() => r(false), 3000)),
      ]);
      if (!exited && !live.exited) live.proc.kill("SIGKILL");
    }
    await live.exitedPromise.catch(() => {});
    // The host process above is confirmed dead, but a crash (or a plain
    // SIGTERM the host never handles) can leave its OWN bun child -- spawned
    // with `.stderr = .inherit` (src/runtime.zig) -- orphaned and still
    // holding the write end of this pipe, so `pumpDone` would never see EOF.
    // Give it a short grace period, then force the pump to stop so close()
    // always resolves rather than hanging on a process we don't own a
    // handle to.
    const drained = await Promise.race([
      live.pumpDone.then(() => true).catch(() => true),
      new Promise<boolean>((r) => setTimeout(() => r(false), STDERR_DRAIN_GRACE_MS)),
    ]);
    if (!drained) {
      live.cancelStderr();
      await live.pumpDone.catch(() => {});
    }
  }

  // --- identity ------------------------------------------------------------

  get pid(): number {
    if (!this.live) throw new Error("AppHandle: host not running");
    return this.live.pid;
  }

  get logPath(): string | undefined {
    return this.config.logPath;
  }

  get socketPath(): string {
    if (!this.live?.socketPath) throw new Error("AppHandle: no automation socket connected");
    return this.live.socketPath;
  }

  get backend(): Backend {
    return this.config.backend;
  }

  get rpc(): TimedClient {
    if (!this.client) throw new Error("AppHandle: not connected");
    return this.client;
  }

  stderr(): string {
    return (this.live?.stderrLines ?? []).join("\n");
  }

  stderrTail(n = 40): string {
    return tail(this.live?.stderrLines ?? [], n);
  }

  // --- tree queries ----------------------------------------------------------

  async tree(window?: number): Promise<GetTreeResult> {
    return this.rpc.call("getTree", { window });
  }

  async find(testId: string, opts: { window?: number } = {}): Promise<JsonNode | null> {
    const t = await this.tree(opts.window);
    return findNode(t.root, testId);
  }

  async findAll(testId: string, opts: { window?: number } = {}): Promise<JsonNode[]> {
    const t = await this.tree(opts.window);
    return findAllNodes(t.root, testId);
  }

  async mustFind(testId: string, opts: { window?: number } = {}): Promise<JsonNode> {
    const node = await this.find(testId, opts);
    if (!node) throw new Error(`${testId} not found in tree`);
    return node;
  }

  async findMatching(pred: (n: JsonNode) => boolean, opts: { window?: number } = {}): Promise<JsonNode | null> {
    const t = await this.tree(opts.window);
    return findMatchingNode(t.root, pred);
  }

  // --- actions (single RPC, host-side resolution, no retry loops) ------------

  click(t: Target): Promise<ClickResult> {
    return this.rpc.call("click", resolveTarget(t));
  }

  setValue(t: Target, value: unknown): Promise<SetValueResult> {
    return this.rpc.call("setValue", { ...resolveTarget(t), value });
  }

  type(t: Target, text: string): Promise<TypeResult> {
    return this.rpc.call("type", { ...resolveTarget(t), text });
  }

  scroll(t: Target, opts: { dx?: number; dy?: number } = {}): Promise<ScrollResult> {
    return this.rpc.call("scroll", { ...resolveTarget(t), ...opts });
  }

  hover(t: Target): Promise<ClickResult> {
    return this.rpc.call("hover", resolveTarget(t));
  }

  doubleClick(t: Target): Promise<ClickResult> {
    return this.rpc.call("doubleClick", resolveTarget(t));
  }

  rightClick(t: Target): Promise<ClickResult> {
    return this.rpc.call("rightClick", resolveTarget(t));
  }

  keys(spec: string, opts: { window?: number } = {}): Promise<KeysResult> {
    return this.rpc.call("keys", { keys: spec, ...opts });
  }

  drag(opts: DragParams): Promise<DragResult> {
    return this.rpc.call("drag", opts);
  }

  // --- waitFor + sugar (thin pass-throughs; the host does the polling) -------

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

  waitForEnabled(testId: string, opts?: WaitOpts): Promise<WaitForResult> {
    return this.waitFor({ testId, state: "enabled" }, opts);
  }

  waitForDisabled(testId: string, opts?: WaitOpts): Promise<WaitForResult> {
    return this.waitFor({ testId, state: "disabled" }, opts);
  }

  waitForFocused(testId: string, opts?: WaitOpts): Promise<WaitForResult> {
    return this.waitFor({ testId, state: "focused" }, opts);
  }

  waitForCount(testId: string, count: number, opts?: WaitOpts): Promise<WaitForResult> {
    return this.waitFor({ testId, countAtLeast: count }, opts);
  }

  waitForValue(testId: string, value: string | number | boolean, opts: WaitOpts & { contains?: boolean } = {}): Promise<WaitForResult> {
    const { contains, ...rest } = opts;
    const rendered = renderWaitValue(value);
    return this.waitFor(contains ? { testId, valueContains: rendered } : { testId, valueEquals: rendered }, rest);
  }

  async waitForMarker(marker: string, timeoutMs = 5000): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      if (this.live?.stderrLines.some((l) => l.includes(marker))) return;
      if (Date.now() >= deadline) {
        throw new Error(`marker "${marker}" not seen within ${timeoutMs}ms\n--- last 40 stderr lines ---\n${this.stderrTail(40)}`);
      }
      await new Promise((r) => setTimeout(r, 50));
    }
  }

  windows(): Promise<WindowsResult> {
    return this.rpc.call("windows");
  }

  async waitForWindows(count: number, timeoutMs = 3000): Promise<number> {
    const result = await poll(() => this.windows(), (r) => r.windows.length === count, { timeoutMs }).catch((e: Error) => {
      throw new Error(`waitForWindows(${count}): ${e.message}`);
    });
    return result.windows.length;
  }

  // --- screenshot --------------------------------------------------------------

  screenshot(path: string, opts: ScreenshotOptions = {}): Promise<ScreenshotResult> {
    return takeScreenshot(
      {
        callScreenshot: (p, window) => this.rpc.call("screenshot", { path: p, window }),
        pid: this.pid,
        backend: this.backend,
      },
      path,
      opts,
    );
  }

  // --- lifecycle ---------------------------------------------------------------

  async restart(): Promise<void> {
    await this.teardown();
    await this.launchWithRetries();
  }

  async close(): Promise<void> {
    await this.teardown();
  }

  kill(): void {
    this.client?.close();
    this.client = undefined;
    if (this.live && !this.live.exited) this.live.proc.kill("SIGKILL");
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }
}

function tail(lines: string[], n: number): string {
  return lines.slice(-n).join("\n");
}

export async function launchApp(opts: LaunchOptions): Promise<AppHandle> {
  const backend = resolveBackend({ backend: opts.backend });
  const binary = opts.hostBinary ?? (await resolveHostBinary({ backend }));
  const config: ResolvedConfig = {
    binary,
    cwd: opts.cwd ?? process.cwd(),
    env: buildEnv(opts),
    readyMarkers: opts.readyMarkers ?? DEFAULT_READY_MARKERS,
    readyTimeoutMs: opts.readyTimeoutMs ?? DEFAULT_READY_TIMEOUT_MS,
    rpcTimeoutMs: opts.rpcTimeoutMs ?? DEFAULT_RPC_TIMEOUT_MS,
    retries: opts.retries ?? DEFAULT_RETRIES,
    backend,
    logPath: opts.logPath,
    onStderr: opts.onStderr,
  };
  return AppHandle.launch(config);
}

export { killAll } from "./registry.ts";
