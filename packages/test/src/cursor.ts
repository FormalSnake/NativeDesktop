// app.cursor: the real system cursor, driven through the host binary's
// `--nd-input` helper (swift/Sources/NDShell/InputHelper.swift), which posts
// HID-level events. Unlike app.mouse, whose `pointer`/`drag` RPCs post
// NSEvents into the app's own queue, these are indistinguishable from a
// physical mouse: hover tracking, native context menus, drag sessions and
// window-server hit testing all see them. The price is the user's cursor: it
// moves, and the app is brought to the front first. macOS only, and the host
// binary needs Accessibility (`<host> --nd-grant` with SIP off, or System
// Settings).
import type { WindowsResult } from "@nativedesktop/react/rpc";
import type { Locator } from "./locator.ts";

export type CursorTarget = Locator | { x: number; y: number };

export interface CursorButtonOptions {
  button?: "left" | "right" | "middle";
  clickCount?: number;
}

export interface CursorMoveOptions {
  /** Intermediate positions on the way, default 12, so hover and drag
   * tracking see a path rather than a jump. */
  steps?: number;
}

interface CursorDeps {
  binary: () => Promise<string>;
  /** The host's pid, to bring it forward before the first event. An attached
   * app does not know it, and relies on the first click to focus instead. */
  pid?: number;
  windows: () => Promise<WindowsResult>;
  /** Window node ref whose top-left `{x, y}` points and locator geometry are
   * relative to; default the first window. */
  window?: number;
}

type Reply = { ok: boolean; error?: string; [k: string]: unknown };

export class Cursor {
  private proc?: ReturnType<typeof Bun.spawn>;
  private lines?: AsyncIterator<string>;
  private queue: Promise<unknown> = Promise.resolve();
  private focused = false;

  constructor(private readonly deps: CursorDeps) {}

  /** Moves to the target's centre (or the window-relative point). */
  async move(target: CursorTarget, opts: CursorMoveOptions = {}): Promise<void> {
    const at = await this.point(target);
    await this.send({ op: "move", ...at, steps: opts.steps ?? 12 });
  }

  hover(target: CursorTarget, opts: CursorMoveOptions = {}): Promise<void> {
    return this.move(target, opts);
  }

  async down(opts: CursorButtonOptions = {}): Promise<void> {
    await this.send({ op: "down", ...opts });
  }

  async up(opts: CursorButtonOptions = {}): Promise<void> {
    await this.send({ op: "up", ...opts });
  }

  async click(target: CursorTarget, opts: CursorButtonOptions & CursorMoveOptions = {}): Promise<void> {
    await this.move(target, opts);
    const clicks = opts.clickCount ?? 1;
    for (let n = 1; n <= clicks; n++) {
      await this.down({ button: opts.button, clickCount: n });
      await this.up({ button: opts.button, clickCount: n });
    }
  }

  dblclick(target: CursorTarget, opts: CursorButtonOptions & CursorMoveOptions = {}): Promise<void> {
    return this.click(target, { ...opts, clickCount: 2 });
  }

  rightClick(target: CursorTarget, opts: CursorMoveOptions = {}): Promise<void> {
    return this.click(target, { ...opts, button: "right" });
  }

  /** Press on `from`, move to `to` through `steps` positions, release. */
  async drag(from: CursorTarget, to: CursorTarget, opts: CursorMoveOptions & { button?: "left" | "right" } = {}): Promise<void> {
    await this.move(from, { steps: opts.steps });
    await this.down({ button: opts.button });
    await this.move(to, { steps: opts.steps ?? 20 });
    await this.up({ button: opts.button });
  }

  /** Scrolls by pixel deltas over the target; positive dy scrolls content up. */
  async scroll(target: CursorTarget, delta: { dx?: number; dy?: number }): Promise<void> {
    await this.move(target, { steps: 1 });
    await this.send({ op: "scroll", dx: delta.dx ?? 0, dy: delta.dy ?? 0 });
  }

  /** The cursor's current position in global logical points. */
  async position(): Promise<{ x: number; y: number }> {
    const r = await this.send({ op: "position" });
    return { x: r.x as number, y: r.y as number };
  }

  close(): void {
    this.proc?.kill();
    this.proc = undefined;
    this.lines = undefined;
    this.focused = false;
  }

  private async point(target: CursorTarget): Promise<{ x: number; y: number }> {
    const { windows } = await this.deps.windows();
    const win = (this.deps.window === undefined ? windows[0] : windows.find((w) => w.ref === this.deps.window)) ?? windows[0];
    if (!win?.geometry) throw new Error("app.cursor: the window reports no geometry (AppKit only)");
    let local: { x: number; y: number };
    if ("boundingBox" in target) {
      const box = await target.boundingBox();
      if (!box) throw new Error("app.cursor: the target has no geometry");
      local = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
    } else {
      local = target;
    }
    return { x: win.geometry.x + local.x, y: win.geometry.y + local.y };
  }

  private send(cmd: Record<string, unknown>): Promise<Reply> {
    const run = this.queue.then(() => this.exchange(cmd));
    this.queue = run.catch(() => {});
    return run;
  }

  private async exchange(cmd: Record<string, unknown>): Promise<Reply> {
    if (!this.proc) {
      if (process.platform !== "darwin") throw new Error("app.cursor is macOS only");
      this.proc = Bun.spawn([await this.deps.binary(), "--nd-input"], { stdin: "pipe", stdout: "pipe", stderr: "inherit" });
      this.lines = readLines(this.proc.stdout as ReadableStream<Uint8Array>);
    }
    if (!this.focused && cmd.op !== "position" && this.deps.pid !== undefined) {
      this.focused = true;
      await this.exchangeRaw({ op: "focus", pid: this.deps.pid });
      // Coming forward (or out of the Stage Manager strip) animates.
      await Bun.sleep(300);
    }
    const reply = await this.exchangeRaw(cmd);
    if (!reply.ok) throw new Error(`app.cursor ${cmd.op}: ${reply.error ?? "failed"}`);
    return reply;
  }

  private async exchangeRaw(cmd: Record<string, unknown>): Promise<Reply> {
    const stdin = this.proc!.stdin as import("bun").FileSink;
    stdin.write(`${JSON.stringify(cmd)}\n`);
    await stdin.flush();
    const next = await this.lines!.next();
    if (next.done) throw new Error("app.cursor: the input helper exited");
    return JSON.parse(next.value) as Reply;
  }
}

async function* readLines(stream: ReadableStream<Uint8Array>): AsyncGenerator<string> {
  const decoder = new TextDecoder();
  let buf = "";
  for await (const chunk of stream) {
    buf += decoder.decode(chunk, { stream: true });
    let nl = buf.indexOf("\n");
    while (nl >= 0) {
      yield buf.slice(0, nl);
      buf = buf.slice(nl + 1);
      nl = buf.indexOf("\n");
    }
  }
}
