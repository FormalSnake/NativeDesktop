// NativeDesktop Protocol (NDP) client library for the Bun runtime child.
// Wire format: u32 LE length prefix + UTF-8 JSON, one frame per message.
// Field names below are a contract with the Zig host (src/protocol.zig) — verbatim, no renaming.
//
// runtime->host families: hello, commitBatch, event, ping, and (M8)
// runtimeError { message, stack } — a best-effort report of an uncaught
// exception / unhandled rejection sent before the process exits, so the
// host's crash overlay shows the real error instead of a bare disconnect.

import { encodeCommitBatchBinary, BinaryUnsupportedValue } from "./ndp-binary";

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
  // Outbound backpressure (Bun sockets don't buffer writes themselves —
  // large frames, e.g. a 100k-row ListView `items` update, can exceed the
  // OS send buffer in one `write()` call; the unwritten remainder must be
  // queued here and flushed from the `drain` handler, or it is silently lost
  // and the Zig reader hangs forever waiting for bytes that never arrive.
  // A FIFO of whole frames (not just one pending buffer) so a second
  // `send()` while an earlier large frame is still draining queues behind
  // it instead of writing concurrently and corrupting frame ordering.
  private outbox: Uint8Array[] = [];
  private outboxOffset = 0;
  // Negotiated CommitBatch encoding (ndp-binary spec §2): default "json";
  // "binary" only if the host advertises it in HelloAck.encodings AND this
  // runtime supports it. Fixed for the connection's lifetime.
  private encoding: "json" | "binary" = "json";

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
        drain(sock) {
          self.pump(sock);
        },
      },
    });
    self = new Ndp(socket);
    return self;
  }

  /// Writes as much of the front-of-queue frame as the socket accepts;
  /// advances to the next frame once the current one is fully written, and
  /// keeps going while the socket keeps accepting bytes. Re-queues (via
  /// `outboxOffset`) whatever remains for the next `drain` event otherwise.
  private pump(sock: import("bun").Socket): void {
    while (this.outbox.length > 0) {
      const front = this.outbox[0]!;
      const remaining = front.subarray(this.outboxOffset);
      const n = sock.write(remaining);
      if (n <= 0) return; // socket full again; wait for the next drain
      this.outboxOffset += n;
      if (this.outboxOffset >= front.length) {
        this.outbox.shift();
        this.outboxOffset = 0;
      } else {
        return; // partial write of the current frame; wait for drain
      }
    }
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
      // Selection rule (spec §2): first host-advertised encoding this runtime
      // supports. This runtime supports "binary"; JSON is always the fallback.
      if (msg.encodings?.includes("binary")) this.encoding = "binary";
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
    const wasEmpty = this.outbox.length === 0;
    this.outbox.push(frame);
    // If nothing was already queued, try to write immediately (the common
    // case: small frames complete in one write()); otherwise an earlier
    // frame is still draining and this one waits its turn in the outbox.
    if (wasEmpty) this.pump(this.socket);
  }

  async handshake(runtime: Runtime): Promise<void> {
    const done = new Promise<void>((res) => (this.helloAckResolve = res));
    this.send({ type: "hello", ndpVersion: NDP_VERSION, runtime });
    await done;
  }

  sendCommit(batch: Omit<CommitBatch, "type">): void {
    if (this.encoding === "binary") {
      // Per-batch fallback (not a per-connection downgrade): a batch whose
      // props have no binary value tag (spec §5.3 — arrays/objects) is sent
      // as a normal JSON frame instead. The host sniffs binary-vs-JSON per
      // frame (isBinaryPayload's magic byte), so mixed streams are legal;
      // `this.encoding` stays "binary" for every later batch.
      try {
        this.sendBinaryFrame(encodeCommitBatchBinary(batch));
        return;
      } catch (err) {
        if (!(err instanceof BinaryUnsupportedValue)) throw err;
        if (TRACE) console.error(`>> [binary fallback to json: ${err.message}]`);
      }
    }
    this.send({ type: "commitBatch", ...batch });
  }

  /// Frames a raw binary payload (u32 LE length prefix + payload) and queues
  /// it through the SAME drain-driven outbox as JSON frames — Bun's socket
  /// does not buffer partial writes, so a large binary CommitBatch must never
  /// bypass the outbox (M5c fact).
  private sendBinaryFrame(payload: Uint8Array): void {
    const frame = new Uint8Array(4 + payload.length);
    new DataView(frame.buffer).setUint32(0, payload.length, true);
    frame.set(payload, 4);
    if (TRACE) console.error(`>> [binary commitBatch ${payload.length}B]`);
    const wasEmpty = this.outbox.length === 0;
    this.outbox.push(frame);
    if (wasEmpty) this.pump(this.socket);
  }

  onEvent(cb: (e: EventMsg) => void): void {
    this.eventCb = cb;
  }

  ping(): void {
    this.send({ type: "ping" });
  }

  /// Best-effort report of an uncaught exception / unhandled rejection,
  /// flushed through the same outbox as commits — sent before the process
  /// dies so the host's overlay (M8) shows the real error, not a bare
  /// disconnect.
  sendRuntimeError(message: string, stack: string): void {
    this.send({ type: "runtimeError", message, stack });
  }
}

export type { Op, CommitBatch, EventMsg, Runtime };
