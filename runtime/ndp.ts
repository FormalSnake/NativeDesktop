// NativeDesktop Protocol (NDP) client library for the Bun runtime child.
// Wire format: u32 LE length prefix + UTF-8 JSON, one frame per message.
// Field names below are a contract with the Zig host (src/protocol.zig) — verbatim, no renaming.

type Runtime = { name: string; version: string };
type Op =
  | { op: "create"; id: number; widget: "Window" | "Box" | "Label" | "Button"; props: Record<string, unknown> }
  | { op: "append"; parent: number; child: number }
  | { op: "insertBefore"; parent: number; child: number; before: number | null }
  | { op: "remove"; id: number }
  | { op: "setText"; id: number; text: string }
  | { op: "update"; id: number; props: Record<string, unknown> }
  | { op: "hide"; id: number }
  | { op: "unhide"; id: number };
type CommitBatch = { type: "commitBatch"; commitId: number; generation: number; ops: Op[] };
type EventMsg = { type: "event"; seq: number; priority: string; nodeId: number; name: string; payload: object };
type HelloAckMsg = { type: "helloAck"; ndpVersion: number; encodings: string[] };
type ErrorMsg = { type: "error"; message: string; expected: number; got: number };
type PongMsg = { type: "pong" };
type InboundMsg = HelloAckMsg | ErrorMsg | EventMsg | PongMsg;

const TRACE = process.env.NDP_TRACE === "1";
const NDP_VERSION = 1;

export class Ndp {
  private socket: import("bun").Socket;
  private inbox = new Uint8Array(0);
  private eventCb: ((e: EventMsg) => void) | null = null;
  private helloAckResolve: (() => void) | null = null;

  private constructor(socket: import("bun").Socket) {
    this.socket = socket;
  }

  static async connect(): Promise<Ndp> {
    const path = process.env.ND_SOCKET;
    if (!path) throw new Error("ND_SOCKET not set");
    let self!: Ndp;
    const socket = await Bun.connect({
      unix: path,
      socket: {
        data(_sock, chunk) {
          self.onData(chunk);
        },
        close() {
          process.exit(0);
        },
      },
    });
    self = new Ndp(socket);
    return self;
  }

  private onData(chunk: Uint8Array): void {
    const merged = new Uint8Array(this.inbox.length + chunk.length);
    merged.set(this.inbox, 0);
    merged.set(chunk, this.inbox.length);
    this.inbox = merged;
    // Drain complete frames; a chunk may contain zero, one, or many frames,
    // and the last frame may be split across the next read.
    while (this.inbox.length >= 4) {
      const view = new DataView(this.inbox.buffer, this.inbox.byteOffset, 4);
      const len = view.getUint32(0, true);
      if (this.inbox.length < 4 + len) break;
      const json = new TextDecoder().decode(this.inbox.subarray(4, 4 + len));
      this.inbox = this.inbox.subarray(4 + len);
      if (TRACE) console.error("<< " + json); // host->runtime = received here
      this.dispatch(JSON.parse(json) as InboundMsg);
    }
  }

  private dispatch(msg: InboundMsg): void {
    if (msg.type === "helloAck") {
      if (msg.ndpVersion !== NDP_VERSION) throw new Error(`ndp mismatch: host ${msg.ndpVersion}`);
      this.helloAckResolve?.();
    } else if (msg.type === "error") {
      throw new Error(`host error: ${msg.message} (expected ${msg.expected}, got ${msg.got})`);
    } else if (msg.type === "event") {
      this.eventCb?.(msg);
    }
    // "pong" carries no payload and needs no handling in M2.
  }

  private send(obj: object): void {
    const json = new TextEncoder().encode(JSON.stringify(obj));
    const frame = new Uint8Array(4 + json.length);
    new DataView(frame.buffer).setUint32(0, json.length, true);
    frame.set(json, 4);
    if (TRACE) console.error(">> " + new TextDecoder().decode(json)); // runtime->host = sent here
    this.socket.write(frame);
  }

  async handshake(runtime: Runtime): Promise<void> {
    const done = new Promise<void>((res) => (this.helloAckResolve = res));
    this.send({ type: "hello", ndpVersion: NDP_VERSION, runtime });
    await done;
  }

  sendCommit(batch: Omit<CommitBatch, "type">): void {
    this.send({ type: "commitBatch", ...batch });
  }

  onEvent(cb: (e: EventMsg) => void): void {
    this.eventCb = cb;
  }

  ping(): void {
    this.send({ type: "ping" });
  }
}

export type { Op, CommitBatch, EventMsg, Runtime };
