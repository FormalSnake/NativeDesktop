// Transport seam for RpcClient: one factory call per dial, WebSocket
// semantics (constructing it starts the connect). The client never touches
// the underlying socket; `close()` must detach every underlying handler
// BEFORE closing, so a dying socket can't deliver late events against the
// fresh connection the client dials next (client.ts invariant (a) lives
// here for the built-in transports).

export interface Transport {
  send(frame: string): void;
  close(): void;
}

export interface TransportHandlers {
  onOpen(): void;
  onMessage(frame: string): void;
  onClose(): void;
}

/** Called once per dial. Constructing it starts the connect (WebSocket semantics).
 * A synchronous throw here is caught by the client and rides the ladder. */
export type TransportFactory = (handlers: TransportHandlers) => Transport;

export function webSocketTransport(url: string, protocols?: string | string[]): TransportFactory {
  return (handlers) => {
    const ws = new WebSocket(url, protocols);
    let closed = false;
    ws.onopen = () => {
      if (!closed) handlers.onOpen();
    };
    ws.onmessage = (ev) => {
      if (!closed) handlers.onMessage(String(ev.data));
    };
    ws.onclose = () => {
      if (closed) return;
      closed = true;
      handlers.onClose();
    };
    ws.onerror = () => {
      /* the close event that follows drives all recovery/rejection logic */
    };
    return {
      send: (frame) => ws.send(frame),
      close: () => {
        closed = true;
        ws.onopen = null;
        ws.onmessage = null;
        ws.onclose = null;
        ws.onerror = null;
        try {
          ws.close();
        } catch {
          // A dead socket may throw on close; the client doesn't care.
        }
      },
    };
  };
}

/** Raw socket transport with NDJSON framing (one JSON frame per newline).
 * `path` dials a unix socket, `host`/`port` a TCP one. */
export function socketTransport(target: { path: string } | { host: string; port: number }): TransportFactory {
  return (handlers) => {
    let closed = false;
    let sock: import("bun").Socket | undefined;
    let buffer = "";
    const decoder = new TextDecoder();
    const encoder = new TextEncoder();
    // Outbound backpressure (the hazard runtime/ndp.ts documents): a Bun
    // socket write can accept fewer bytes than given (or -1 while closing),
    // and the unwritten remainder must be queued and flushed from `drain`,
    // or it is silently lost and the NDJSON stream is mis-framed from that
    // point on. A FIFO of whole frames — byte-encoded up front so the
    // offset arithmetic survives multi-byte UTF-8 — keeps a second send()
    // ordered behind a still-draining large frame.
    const outbox: Uint8Array[] = [];
    let outboxOffset = 0;
    const pump = (s: import("bun").Socket): void => {
      while (outbox.length > 0) {
        const front = outbox[0]!;
        const n = s.write(front.subarray(outboxOffset));
        if (n <= 0) return; // buffer full (or closing); wait for the next drain
        outboxOffset += n;
        if (outboxOffset >= front.length) {
          outbox.shift();
          outboxOffset = 0;
        } else {
          return; // partial write; the rest goes out on drain
        }
      }
    };
    const emitClose = (): void => {
      if (closed) return;
      closed = true;
      handlers.onClose();
    };
    const socketHandlers = {
      open(s: import("bun").Socket) {
        sock = s;
        if (closed) s.end();
        else handlers.onOpen();
      },
      data(_s: import("bun").Socket, chunk: Uint8Array) {
        if (closed) return;
        buffer += decoder.decode(chunk, { stream: true });
        let newline: number;
        while (!closed && (newline = buffer.indexOf("\n")) >= 0) {
          const line = buffer.slice(0, newline);
          buffer = buffer.slice(newline + 1);
          if (line.length > 0) handlers.onMessage(line);
        }
      },
      drain(s: import("bun").Socket) {
        if (!closed) pump(s);
      },
      close() {
        emitClose();
      },
      error() {
        /* the close event that follows drives all recovery/rejection logic */
      },
      connectError() {
        emitClose();
      },
    };
    const connecting =
      "path" in target
        ? Bun.connect({ unix: target.path, socket: socketHandlers })
        : Bun.connect({ hostname: target.host, port: target.port, socket: socketHandlers });
    connecting.catch(() => emitClose());
    return {
      send: (frame) => {
        if (closed || !sock) return;
        outbox.push(encoder.encode(frame + "\n"));
        pump(sock);
      },
      close: () => {
        closed = true;
        sock?.end();
      },
    };
  };
}
