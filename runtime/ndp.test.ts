// Standalone verification for runtime/ndp.ts against a mock unix-socket host,
// since the Zig host (src/protocol.zig, src/runtime.zig) does not exist yet.
// Run with: nix develop -c bun test runtime/

import { test, expect, afterAll } from "bun:test";
import { Ndp } from "./ndp";

function frame(json: string): Uint8Array {
  const body = new TextEncoder().encode(json);
  const buf = new Uint8Array(4 + body.length);
  new DataView(buf.buffer).setUint32(0, body.length, true);
  buf.set(body, 4);
  return buf;
}

function decodeFrames(bytes: Uint8Array): string[] {
  const out: string[] = [];
  let offset = 0;
  while (offset + 4 <= bytes.length) {
    const len = new DataView(bytes.buffer, bytes.byteOffset + offset, 4).getUint32(0, true);
    const start = offset + 4;
    out.push(new TextDecoder().decode(bytes.subarray(start, start + len)));
    offset = start + len;
  }
  return out;
}

// Ndp's close() handler calls process.exit(0) — correct for the real child
// process, but fatal to the test runner if a mock server closes a client
// socket mid-suite. Keep every server (and thus every client connection)
// alive until the whole file is done, torn down once via afterAll.
const servers: ReturnType<typeof Bun.listen>[] = [];
afterAll(() => {
  for (const s of servers) s.stop(true);
});

test("golden frame: u32 LE length prefix + json byte layout", () => {
  const json = JSON.stringify({ type: "hello", ndpVersion: 1, runtime: { name: "bun", version: "1.3.13" } });
  const f = frame(json);
  const len = new DataView(f.buffer).getUint32(0, true);
  expect(len).toBe(json.length);
  expect(f.length).toBe(4 + json.length);
  expect(f[4]).toBe("{".charCodeAt(0));
  // Length low byte first (little-endian) — for this payload size, low byte equals the full length.
  expect(f[0]).toBe(json.length & 0xff);
});

test("handshake: hello/helloAck round-trip over a mock unix host", async () => {
  const sockPath = `/tmp/ndp-test-${process.pid}-${Date.now()}.sock`;
  let receivedHello: any = null;

  const server = Bun.listen({
    unix: sockPath,
    socket: {
      open() {},
      data(sock, chunk) {
        const [json] = decodeFrames(chunk);
        receivedHello = JSON.parse(json!);
        sock.write(frame(JSON.stringify({ type: "helloAck", ndpVersion: 1, encodings: ["json"] })));
      },
      close() {},
    },
  });
  servers.push(server);

  process.env.ND_SOCKET = sockPath;
  const ndp = await Ndp.connect();
  await ndp.handshake({ name: "bun", version: "1.3.13" });

  expect(receivedHello.type).toBe("hello");
  expect(receivedHello.ndpVersion).toBe(1);
  expect(receivedHello.runtime).toEqual({ name: "bun", version: "1.3.13" });
});

test("partial-frame reassembly: a frame split across multiple socket reads", async () => {
  const sockPath = `/tmp/ndp-test-${process.pid}-${Date.now()}-split.sock`;
  const events: any[] = [];

  const server = Bun.listen({
    unix: sockPath,
    socket: {
      open(sock) {
        // Immediately ack the hello so handshake() resolves, then dribble an
        // event frame out in three separate writes: part of the length prefix,
        // the rest of the prefix, then the body in two more pieces.
        sock.write(frame(JSON.stringify({ type: "helloAck", ndpVersion: 1, encodings: ["json"] })));

        const eventJson = JSON.stringify({
          type: "event",
          seq: 1,
          priority: "discrete",
          nodeId: 4,
          name: "clicked",
          payload: {},
        });
        const full = frame(eventJson);
        queueMicrotask(async () => {
          // Split at byte 2 (mid length-prefix), then mid-body, to prove
          // reassembly does not assume length-prefix or body arrive whole.
          const cut1 = 2;
          const cut2 = Math.floor(full.length * 0.6);
          sock.write(full.subarray(0, cut1));
          await Bun.sleep(5);
          sock.write(full.subarray(cut1, cut2));
          await Bun.sleep(5);
          sock.write(full.subarray(cut2));
        });
      },
      data() {},
      close() {},
    },
  });
  servers.push(server);

  process.env.ND_SOCKET = sockPath;
  const ndp = await Ndp.connect();
  await ndp.handshake({ name: "bun", version: "1.3.13" });

  const gotEvent = new Promise<void>((resolve) => {
    ndp.onEvent((e) => {
      events.push(e);
      resolve();
    });
  });
  await gotEvent;

  expect(events.length).toBe(1);
  expect(events[0]).toEqual({ type: "event", seq: 1, priority: "discrete", nodeId: 4, name: "clicked", payload: {} });
});

test("sendCommit: commitBatch frame matches the wire contract field-for-field", async () => {
  const sockPath = `/tmp/ndp-test-${process.pid}-${Date.now()}-commit.sock`;
  let receivedCommit: any = null;
  const gotCommit = Promise.withResolvers<void>();

  const server = Bun.listen({
    unix: sockPath,
    socket: {
      open(sock) {
        sock.write(frame(JSON.stringify({ type: "helloAck", ndpVersion: 1, encodings: ["json"] })));
      },
      data(_sock, chunk) {
        const frames = decodeFrames(chunk);
        for (const f of frames) {
          const msg = JSON.parse(f);
          if (msg.type === "commitBatch") {
            receivedCommit = msg;
            gotCommit.resolve();
          }
        }
      },
      close() {},
    },
  });
  servers.push(server);

  process.env.ND_SOCKET = sockPath;
  const ndp = await Ndp.connect();
  await ndp.handshake({ name: "bun", version: "1.3.13" });

  ndp.sendCommit({
    commitId: 0,
    generation: 0,
    ops: [
      { op: "create", id: 1, widget: "Window", props: { title: "Hi", defaultWidth: 480, defaultHeight: 320 } },
      { op: "create", id: 2, widget: "Box", props: { orientation: "vertical", spacing: 8 } },
      { op: "append", parent: 1, child: 2 },
      { op: "create", id: 3, widget: "Label", props: { text: "Clicks: 0" } },
      { op: "setText", id: 3, text: "Clicks: 1" },
    ],
  });

  await gotCommit.promise;

  expect(receivedCommit.type).toBe("commitBatch");
  expect(receivedCommit.commitId).toBe(0);
  expect(receivedCommit.generation).toBe(0);
  expect(receivedCommit.ops.length).toBe(5);
  expect(receivedCommit.ops[0].op).toBe("create");
  expect(receivedCommit.ops[0].widget).toBe("Window");
  expect(receivedCommit.ops[2].child).toBe(2);
  expect(receivedCommit.ops[4].text).toBe("Clicks: 1");
});
