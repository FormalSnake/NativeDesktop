// Pins socketTransport's drain-driven outbox: Bun's socket.write accepts
// fewer bytes than given under backpressure, and the remainder must survive
// to the next drain with byte-exact framing (a dropped tail mis-frames the
// NDJSON stream for the rest of the connection).

import { describe, expect, test } from "bun:test";
import { socketTransport } from "./transport.ts";

describe("socketTransport", () => {
  test("a frame larger than the send buffer arrives intact, in order", async () => {
    const lines: string[] = [];
    let lineBuffer = "";
    const decoder = new TextDecoder();
    const gotTwo = Promise.withResolvers<void>();
    const server = Bun.listen({
      hostname: "127.0.0.1",
      port: 0,
      socket: {
        data(_s, chunk) {
          lineBuffer += decoder.decode(chunk, { stream: true });
          let newline: number;
          while ((newline = lineBuffer.indexOf("\n")) >= 0) {
            lines.push(lineBuffer.slice(0, newline));
            lineBuffer = lineBuffer.slice(newline + 1);
          }
          if (lines.length >= 2) gotTwo.resolve();
        },
      },
    });
    const opened = Promise.withResolvers<void>();
    const transport = socketTransport({ host: "127.0.0.1", port: server.port })({
      onOpen: () => opened.resolve(),
      onMessage: () => {},
      onClose: () => {},
    });
    await opened.promise;
    // Multi-byte characters make the frame's byte length differ from its
    // string length, so a character-based outbox offset would corrupt it.
    const big = JSON.stringify({ body: "π".repeat(2 * 1024 * 1024) });
    transport.send(big);
    transport.send("tail"); // queues behind the still-draining big frame
    await gotTwo.promise;
    expect(lines[0]).toBe(big);
    expect(lines[1]).toBe("tail");
    transport.close();
    server.stop();
  });
});
