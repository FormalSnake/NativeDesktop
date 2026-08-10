// Test double for the Transport seam (not exported from index.ts). Mimics
// the built-in transports' contract exactly: events fire asynchronously
// never during the factory call, and nothing is delivered after the client
// calls close().

import type { Transport, TransportFactory, TransportHandlers } from "./transport.ts";

export interface FakeFrame {
  jsonrpc: "2.0";
  id?: number;
  method?: string;
  params?: unknown;
}

export class FakeConn {
  readonly sent: string[] = [];
  clientClosed = false;
  #handlers: TransportHandlers;
  #serverClosed = false;

  constructor(handlers: TransportHandlers) {
    this.#handlers = handlers;
  }

  get transport(): Transport {
    return {
      send: (frame) => {
        if (!this.clientClosed && !this.#serverClosed) this.sent.push(frame);
      },
      close: () => {
        this.clientClosed = true;
      },
    };
  }

  get closed(): boolean {
    return this.clientClosed || this.#serverClosed;
  }

  /** Frames the client sent, parsed. */
  calls(): FakeFrame[] {
    return this.sent.map((s) => JSON.parse(s) as FakeFrame);
  }

  lastCall(): FakeFrame | undefined {
    return this.calls().at(-1);
  }

  callFor(method: string): FakeFrame | undefined {
    return this.calls().find((c) => c.method === method);
  }

  open(): void {
    if (!this.closed) this.#handlers.onOpen();
  }

  deliver(obj: unknown): void {
    if (!this.closed) this.#handlers.onMessage(JSON.stringify(obj));
  }

  reply(id: number, result: unknown): void {
    this.deliver({ jsonrpc: "2.0", id, result });
  }

  replyError(id: number, code: number, message: string, data?: unknown): void {
    this.deliver({ jsonrpc: "2.0", id, error: { code, message, data } });
  }

  notify(method: string, params: unknown): void {
    this.deliver({ jsonrpc: "2.0", method, params });
  }

  /** Server-side drop (RST/close). Inert after the client already closed. */
  drop(): void {
    if (this.closed) return;
    this.#serverClosed = true;
    this.#handlers.onClose();
  }
}

export interface FakeTransport {
  factory: TransportFactory;
  conns: FakeConn[];
  latest(): FakeConn;
}

export function fakeTransport(): FakeTransport {
  const conns: FakeConn[] = [];
  return {
    factory: (handlers) => {
      const conn = new FakeConn(handlers);
      conns.push(conn);
      return conn.transport;
    },
    conns,
    latest: () => {
      if (conns.length === 0) throw new Error("no fake connection dialed yet");
      return conns[conns.length - 1]!;
    },
  };
}
