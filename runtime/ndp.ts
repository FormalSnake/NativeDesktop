// NativeDesktop Protocol (NDP) client library for the Bun runtime child.
// Wire format: u32 LE length prefix + UTF-8 JSON, one frame per message.
// Field names below are a contract with the Zig host (src/protocol.zig) — verbatim, no renaming.
//
// runtime->host families: hello, commitBatch, event, ping, and (M8)
// runtimeError { message, stack, fatal }, a best-effort report of an
// uncaught error. fatal=true precedes a process exit (crash overlay shows
// the text); fatal=false reports a survived error (host logs it, no overlay).

import { encodeCommitBatchBinary, BinaryUnsupportedValue } from "./ndp-binary";
// Message shapes are GENERATED from schema/protocol.json (the single source
// of truth shared with the Zig mirror, src/generated/protocol.zig) — a field
// rename or type change there regenerates both sides, so drift is a compile
// error, not a silent wire break.
import { NDP_VERSION } from "../packages/react/src/generated/protocol";
import type { Runtime, Op, CommitBatch, EventMsg, HostToRuntimeMsg } from "../packages/react/src/generated/protocol";

interface PendingRequest {
  resolve: (result: unknown) => void;
  reject: (err: Error) => void;
}

const TRACE = process.env.NDP_TRACE === "1";

// One decoder for the process: TextDecoder carries no per-frame state, and a
// fresh one per frame showed up as pure allocation on getTree-sized replies.
const textDecoder = new TextDecoder();
const textEncoder = new TextEncoder();

export class Ndp {
  private socket: import("bun").Socket;
  // Inbound buffer: bytes between `inboxHead` (first unread) and `inboxTail`
  // (one past the last received) are a partial frame stream. Reading advances
  // the head rather than reallocating, so a multi-MB reply arriving in 64 KB
  // chunks costs one copy per chunk instead of the n^2/2 the old
  // `new Uint8Array(inbox + chunk)` per chunk cost.
  private inbox = new Uint8Array(64 * 1024);
  private inboxView = new DataView(this.inbox.buffer);
  private inboxHead = 0;
  private inboxTail = 0;
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
  // Active host widget backend, learned from the helloAck ("gtk" | "appkit").
  // "unknown" until the handshake completes.
  private backendName = "unknown";
  // Host capability manifest (helloAck hostWidgets/hostCommands). null when
  // the host predates the fields — callers fall back to their own schema
  // tables (see packages/react/src/platform.ts's setHostManifest).
  private hostWidgetsSet: Set<string> | null = null;
  private hostCommandsSet: Set<string> | null = null;
  // systemRequest/systemResponse correlation (system capabilities API, M15):
  // one entry per in-flight request, keyed by the id sent on the wire.
  private pending = new Map<number, PendingRequest>();
  private nextRequestId = 1;
  private systemEventCb: ((channel: string, data: unknown) => void) | null = null;

  private constructor(socket: import("bun").Socket) {
    this.socket = socket;
  }

  /** The host's active widget backend, valid after `handshake()` resolves. */
  get backend(): string {
    return this.backendName;
  }

  /** Intrinsics the host build knows, or null on a pre-manifest host. Valid after `handshake()`. */
  get hostWidgets(): Set<string> | null {
    return this.hostWidgetsSet;
  }

  /** "<intrinsic>.<command>" entries the host dispatches, or null on a pre-manifest host. Valid after `handshake()`. */
  get hostCommands(): Set<string> | null {
    return this.hostCommandsSet;
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
          // A dying connection must not leave callers of request() awaiting
          // forever; reject whatever is in flight before the process exits.
          for (const call of self.pending.values()) call.reject(new Error("ndp connection closed"));
          self.pending.clear();
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

  /// Makes room for `extra` more bytes after the tail: first by sliding the
  /// unread bytes down over the consumed head, and only if that is not enough
  /// by doubling into a larger buffer.
  private reserveInbox(extra: number): void {
    const pending = this.inboxTail - this.inboxHead;
    if (this.inboxTail + extra <= this.inbox.length) return;
    if (pending + extra <= this.inbox.length) {
      this.compactInbox();
      return;
    }
    let cap = this.inbox.length * 2;
    while (cap < pending + extra) cap *= 2;
    const next = new Uint8Array(cap);
    next.set(this.inbox.subarray(this.inboxHead, this.inboxTail));
    this.inbox = next;
    this.inboxView = new DataView(next.buffer);
    this.inboxHead = 0;
    this.inboxTail = pending;
  }

  private compactInbox(): void {
    if (this.inboxHead === 0) return;
    this.inbox.copyWithin(0, this.inboxHead, this.inboxTail);
    this.inboxTail -= this.inboxHead;
    this.inboxHead = 0;
  }

  private onData(chunk: Uint8Array): void {
    this.reserveInbox(chunk.length);
    this.inbox.set(chunk, this.inboxTail);
    this.inboxTail += chunk.length;
    // Drain complete frames; a chunk may contain zero, one, or many frames,
    // and the last frame may be split across the next read.
    while (this.inboxTail - this.inboxHead >= 4) {
      const len = this.inboxView.getUint32(this.inboxHead, true);
      if (this.inboxTail - this.inboxHead < 4 + len) break;
      const start = this.inboxHead + 4;
      const json = textDecoder.decode(this.inbox.subarray(start, start + len));
      this.inboxHead = start + len;
      if (TRACE) console.error("<< " + json); // host->runtime = received here
      this.dispatch(JSON.parse(json) as HostToRuntimeMsg);
    }
    // Reclaim the consumed head only once it is worth the memmove, so a steady
    // stream of small frames does not slide the remainder down on every chunk.
    if (this.inboxHead === this.inboxTail) {
      this.inboxHead = 0;
      this.inboxTail = 0;
    } else if (this.inboxHead * 2 >= this.inbox.length) {
      this.compactInbox();
    }
  }

  private dispatch(msg: HostToRuntimeMsg): void {
    if (msg.type === "helloAck") {
      if (msg.ndpVersion !== NDP_VERSION) throw new Error(`ndp mismatch: host ${msg.ndpVersion}`);
      // Selection rule (spec §2): first host-advertised encoding this runtime
      // supports. This runtime supports "binary"; JSON is always the fallback.
      // ND_FORCE_JSON=1 (M10 bench) makes the runtime ignore an advertised
      // "binary" so the JSON leg can be measured against the same host build.
      if (msg.encodings?.includes("binary") && process.env.ND_FORCE_JSON !== "1") this.encoding = "binary";
      this.backendName = msg.backend;
      this.hostWidgetsSet = msg.hostWidgets ? new Set(msg.hostWidgets) : null;
      this.hostCommandsSet = msg.hostCommands ? new Set(msg.hostCommands) : null;
      this.helloAckResolve?.();
    } else if (msg.type === "error") {
      throw new Error(`host error: ${msg.message} (expected ${msg.expected}, got ${msg.got})`);
    } else if (msg.type === "event") {
      this.eventCb?.(msg);
    } else if (msg.type === "systemResponse") {
      const call = this.pending.get(msg.id);
      if (!call) return; // unknown/already-settled id — nothing to do
      this.pending.delete(msg.id);
      if (msg.ok) call.resolve(msg.result);
      else call.reject(new Error(msg.errorMessage ?? "system request failed"));
    } else if (msg.type === "systemEvent") {
      this.systemEventCb?.(msg.channel, msg.data);
    }
    // "pong" carries no payload and needs no handling in M2.
  }

  private send(obj: object): void {
    const json = textEncoder.encode(JSON.stringify(obj));
    const frame = new Uint8Array(4 + json.length);
    new DataView(frame.buffer).setUint32(0, json.length, true);
    frame.set(json, 4);
    if (TRACE) console.error(">> " + textDecoder.decode(json)); // runtime->host = sent here
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

  /// Imperative command on a live widget node (widgetCommand frame, M14 —
  /// e.g. WebView goBack/reload). Rides the same FIFO outbox as commits, so
  /// a command sent after a commit is applied after that commit host-side.
  sendWidgetCommand(nodeId: number, command: string, arg: unknown = null): void {
    this.send({ type: "widgetCommand", nodeId, command, arg });
  }

  onEvent(cb: (e: EventMsg) => void): void {
    this.eventCb = cb;
  }

  /// Sends a systemRequest for a native capability method (e.g.
  /// "dialog.openFile") and resolves/rejects with the matching
  /// systemResponse (correlated by id, settled in dispatch()). Rejects if
  /// the connection closes before a reply arrives.
  request(method: string, params: unknown): Promise<unknown> {
    const id = this.nextRequestId++;
    return new Promise<unknown>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.send({ type: "systemRequest", id, method, params: params ?? {} });
    });
  }

  /// Registers the callback for host-originated systemEvent frames (app
  /// activation/deactivation, OS open-url/open-file, notification clicks,
  /// file drops). One callback for the connection's lifetime, same shape as
  /// onEvent().
  onSystemEvent(cb: (channel: string, data: unknown) => void): void {
    this.systemEventCb = cb;
  }

  ping(): void {
    this.send({ type: "ping" });
  }

  /// Best-effort error report, flushed through the same outbox as commits.
  /// fatal=true is sent just before the process dies so the host's overlay
  /// (M8) shows the real error, not a bare disconnect; fatal=false reports
  /// a survived error the host logs without stashing.
  sendRuntimeError(message: string, stack: string, fatal: boolean): void {
    this.send({ type: "runtimeError", message, stack, fatal });
  }
}

export type { Op, CommitBatch, EventMsg, Runtime };
