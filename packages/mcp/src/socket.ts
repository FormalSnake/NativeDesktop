// Framed JSON-RPC 2.0 client for the NativeDesktop automation socket.
// Wire format matches NDP: u32 LE length prefix + UTF-8 JSON (runtime/ndp.ts),
// but the payload here is a JSON-RPC 2.0 request/response, not an NDP message.
// Field names are a contract with the Zig host (src/automation.zig) — verbatim, no renaming.

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
    if (msg.error) pending.reject(new Error(`${msg.error.message} (${msg.error.code})`));
    else pending.resolve(msg.result);
  }

  call(method: string, params?: unknown): Promise<unknown> {
    const id = this.nextId++;
    const json = new TextEncoder().encode(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
    const frame = new Uint8Array(4 + json.length);
    new DataView(frame.buffer).setUint32(0, json.length, true);
    frame.set(json, 4);
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.write(frame);
    });
  }

  close(): void {
    this.socket.end();
  }
}
