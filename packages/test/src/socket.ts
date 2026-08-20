// Framed JSON-RPC 2.0 client for the NativeDesktop automation socket.
// Wire format matches NDP: u32 LE length prefix + UTF-8 JSON (runtime/ndp.ts),
// but the payload here is a JSON-RPC 2.0 request/response, not an NDP message.
// Method names and params/result shapes are GENERATED from schema/rpc.json
// (the single source of truth shared with the Zig host, src/automation.zig via
// src/generated/rpc.zig) — `call` is constrained by the generated RpcMethods
// map, so a schema change is a compile error here, tRPC-style.

import type { RpcMethodName, RpcParams, RpcResult } from "@nativedesktop/react/rpc";

interface JsonRpcError {
  code: number;
  message: string;
  data?: unknown;
}

interface JsonRpcResponse {
  jsonrpc: "2.0";
  id: number;
  result?: unknown;
  error?: JsonRpcError;
}

/**
 * A rejected call's `code`/`data` survive on the error object (matching
 * schema/rpc.json's `errors` table, e.g. RPC_ERRORS.windowGone.code), so a
 * caller can branch on the error kind without parsing `.message` text.
 * `.message` keeps the existing "<msg> (<code>)" shape so callers that
 * already substring-match a code keep working.
 */
export class AutomationRpcError extends Error {
  readonly code: number;
  readonly data: unknown;

  constructor(code: number, message: string, data?: unknown) {
    super(`${message} (${code})`);
    this.name = "AutomationRpcError";
    this.code = code;
    this.data = data;
  }
}

export class AutomationClient {
  private socket!: import("bun").Socket;
  private inbox = new Uint8Array(0);
  private nextId = 1;
  private pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();

  static async connect(path = process.env.ND_AUTOMATION_SOCKET): Promise<AutomationClient> {
    if (!path) throw new Error("ND_AUTOMATION_SOCKET not set");
    let self!: AutomationClient;
    const socket = await Bun.connect({
      unix: path,
      socket: {
        data(_sock, chunk) {
          self.onData(chunk);
        },
        close() {
          self.onClose();
        },
      },
    });
    self = new AutomationClient(socket);
    return self;
  }

  private constructor(socket: import("bun").Socket) {
    this.socket = socket;
  }

  private onClose(): void {
    for (const p of this.pending.values()) p.reject(new Error("automation socket closed"));
    this.pending.clear();
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
      this.dispatch(JSON.parse(json) as JsonRpcResponse);
    }
  }

  private dispatch(msg: JsonRpcResponse): void {
    const pending = this.pending.get(msg.id);
    if (!pending) return; // unknown/stale id — drop
    this.pending.delete(msg.id);
    if (msg.error) pending.reject(new AutomationRpcError(msg.error.code, msg.error.message, msg.error.data));
    else pending.resolve(msg.result);
  }

  call<M extends RpcMethodName>(
    method: M,
    ...params: RpcParams<M> extends undefined ? [] : [RpcParams<M>]
  ): Promise<RpcResult<M>> {
    const id = this.nextId++;
    const json = new TextEncoder().encode(JSON.stringify({ jsonrpc: "2.0", id, method, params: params[0] }));
    const frame = new Uint8Array(4 + json.length);
    new DataView(frame.buffer).setUint32(0, json.length, true);
    frame.set(json, 4);
    return new Promise<RpcResult<M>>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      this.socket.write(frame);
    });
  }

  close(): void {
    this.socket.end();
  }
}
