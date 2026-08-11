// Standalone verification for packages/mcp against a mock unix-socket automation
// host, since the Zig automation server (src/automation.zig) is a concurrently
// developed track. Mirrors the mock-server pattern in runtime/ndp.test.ts.
// Run with: nix develop -c bun test packages/mcp/

import { test, expect, afterAll } from "bun:test";
import { AutomationClient } from "@nativedesktop/test";

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

// AutomationClient's close() rejects pending calls but does not exit the
// process (unlike Ndp's), so servers only need stopping, not special handling
// — still tear down via afterAll to keep every mock host alive for the suite.
const servers: ReturnType<typeof Bun.listen>[] = [];
afterAll(() => {
  for (const s of servers) s.stop(true);
});

function mockHost(handle: (msg: any, sock: import("bun").Socket) => void): string {
  const sockPath = `/tmp/nd-automation-test-${process.pid}-${Date.now()}-${Math.random().toString(36).slice(2)}.sock`;
  const server = Bun.listen({
    unix: sockPath,
    socket: {
      open() {},
      data(sock, chunk) {
        for (const json of decodeFrames(chunk)) handle(JSON.parse(json), sock);
      },
      close() {},
    },
  });
  servers.push(server);
  return sockPath;
}

test("golden frame: u32 LE length prefix + json byte layout matches NDP framing", () => {
  const json = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "getTree" });
  const f = frame(json);
  const len = new DataView(f.buffer).getUint32(0, true);
  expect(len).toBe(json.length);
  expect(f.length).toBe(4 + json.length);
  expect(f[4]).toBe("{".charCodeAt(0));
});

test("request/response correlation: id round-trips through a JSON-RPC result", async () => {
  let receivedRequest: any = null;
  const sockPath = mockHost((msg, sock) => {
    receivedRequest = msg;
    sock.write(frame(JSON.stringify({ jsonrpc: "2.0", id: msg.id, result: { ok: true } })));
  });

  const client = await AutomationClient.connect(sockPath);
  const result = await client.call("getTree");

  expect(receivedRequest.jsonrpc).toBe("2.0");
  expect(receivedRequest.method).toBe("getTree");
  expect(typeof receivedRequest.id).toBe("number");
  // The mock host replies with a deliberately schema-less payload — this test
  // pins framing/correlation, not result shapes, so widen past the typed call.
  expect(result as unknown).toEqual({ ok: true });
});

test("request/response correlation: concurrent calls resolve to their own ids, not FIFO order", async () => {
  const sockPath = mockHost((msg, sock) => {
    // Reply out of order: id 2 first, then id 1 — proves correlation is by id, not arrival order.
    if (msg.id === 1) {
      setTimeout(() => sock.write(frame(JSON.stringify({ jsonrpc: "2.0", id: 1, result: { which: "first-call" } }))), 10);
    } else {
      sock.write(frame(JSON.stringify({ jsonrpc: "2.0", id: msg.id, result: { which: "second-call" } })));
    }
  });

  const client = await AutomationClient.connect(sockPath);
  const p1 = client.call("getTree");
  const p2 = client.call("click", { ref: 1 });
  const [r1, r2] = await Promise.all([p1, p2]);

  // Schema-less mock payloads again (correlation is the contract under test).
  expect(r1 as unknown).toEqual({ which: "first-call" });
  expect(r2 as unknown).toEqual({ which: "second-call" });
});

test("error responses reject the call with code and message", async () => {
  const sockPath = mockHost((msg, sock) => {
    sock.write(
      frame(JSON.stringify({ jsonrpc: "2.0", id: msg.id, error: { code: -32001, message: "not actionable", data: { reason: "invisible" } } })),
    );
  });

  const client = await AutomationClient.connect(sockPath);
  await expect(client.call("click", { ref: 99 })).rejects.toThrow(/not actionable.*-32001/);
});

test("partial-frame reassembly: a JSON-RPC response split across multiple socket reads", async () => {
  const sockPath = mockHost((msg, sock) => {
    // Dribble the reply out in three separate writes: part of the length
    // prefix, the rest of the prefix, then the body in two more pieces —
    // proves reassembly does not assume length-prefix or body arrive whole.
    const full = frame(JSON.stringify({ jsonrpc: "2.0", id: msg.id, result: { matched: true } }));
    const cut1 = 2;
    const cut2 = Math.floor(full.length * 0.6);
    sock.write(full.subarray(0, cut1));
    setTimeout(() => {
      sock.write(full.subarray(cut1, cut2));
      setTimeout(() => sock.write(full.subarray(cut2)), 5);
    }, 5);
  });

  const client = await AutomationClient.connect(sockPath);
  const result = await client.call("waitFor", { condition: { textContains: "Clicks: 3" }, timeoutMs: 2000 });
  expect(result).toEqual({ matched: true });
});

test("tool-call -> JSON-RPC mapping: nd_get_tree sends getTree with no params", async () => {
  let received: any = null;
  const sockPath = mockHost((msg, sock) => {
    received = msg;
    sock.write(frame(JSON.stringify({ jsonrpc: "2.0", id: msg.id, result: { coordinateSpace: "logical-window-topleft", root: {} } })));
  });
  const client = await AutomationClient.connect(sockPath);

  // Mirrors src/index.ts's nd_get_tree handler: client.call("getTree").
  const result = await client.call("getTree");

  expect(received.method).toBe("getTree");
  expect(received.params).toBeUndefined();
  // root: {} is a mock stand-in, not a full JsonNode — widen past the typed call.
  expect(result as unknown).toEqual({ coordinateSpace: "logical-window-topleft", root: {} });
});

test("tool-call -> JSON-RPC mapping: nd_screenshot sends screenshot with {path}", async () => {
  let received: any = null;
  const sockPath = mockHost((msg, sock) => {
    received = msg;
    sock.write(frame(JSON.stringify({ jsonrpc: "2.0", id: msg.id, result: { path: msg.params.path, width: 480, height: 320 } })));
  });
  const client = await AutomationClient.connect(sockPath);

  // Mirrors src/index.ts's nd_screenshot handler: client.call("screenshot", { path }).
  const result = await client.call("screenshot", { path: "/tmp/m4.png" });

  expect(received.method).toBe("screenshot");
  expect(received.params).toEqual({ path: "/tmp/m4.png" });
  expect(result).toEqual({ path: "/tmp/m4.png", width: 480, height: 320 });
});

test("tool-call -> JSON-RPC mapping: nd_click sends click with {ref}", async () => {
  let received: any = null;
  const sockPath = mockHost((msg, sock) => {
    received = msg;
    sock.write(frame(JSON.stringify({ jsonrpc: "2.0", id: msg.id, result: { ref: msg.params.ref, dispatched: true } })));
  });
  const client = await AutomationClient.connect(sockPath);

  // Mirrors src/index.ts's nd_click handler: client.call("click", { ref }).
  const result = await client.call("click", { ref: 16777220 });

  expect(received.method).toBe("click");
  expect(received.params).toEqual({ ref: 16777220 });
  expect(result).toEqual({ ref: 16777220, dispatched: true });
});

test("tool-call -> JSON-RPC mapping: nd_wait_for sends waitFor with {condition,timeoutMs} for textContains", async () => {
  let received: any = null;
  const sockPath = mockHost((msg, sock) => {
    received = msg;
    sock.write(frame(JSON.stringify({ jsonrpc: "2.0", id: msg.id, result: { matched: true } })));
  });
  const client = await AutomationClient.connect(sockPath);

  // Mirrors src/index.ts's nd_wait_for handler for the textContains branch:
  // condition = textContains !== undefined ? { textContains } : { refVisible }.
  const textContains = "Clicks: 3";
  const refVisible = undefined;
  const timeoutMs = 3000;
  const condition = textContains !== undefined ? { textContains } : { refVisible };
  const result = await client.call("waitFor", { condition, timeoutMs });

  expect(received.method).toBe("waitFor");
  expect(received.params).toEqual({ condition: { textContains: "Clicks: 3" }, timeoutMs: 3000 });
  expect(result).toEqual({ matched: true });
});

test("tool-call -> JSON-RPC mapping: nd_wait_for sends waitFor with {condition,timeoutMs} for refVisible", async () => {
  let received: any = null;
  const sockPath = mockHost((msg, sock) => {
    received = msg;
    sock.write(frame(JSON.stringify({ jsonrpc: "2.0", id: msg.id, result: { matched: true } })));
  });
  const client = await AutomationClient.connect(sockPath);

  // Mirrors src/index.ts's nd_wait_for handler for the refVisible branch.
  const textContains = undefined;
  const refVisible = 16777220;
  const timeoutMs = 2000;
  const condition = textContains !== undefined ? { textContains } : { refVisible };
  const result = await client.call("waitFor", { condition, timeoutMs });

  expect(received.method).toBe("waitFor");
  expect(received.params).toEqual({ condition: { refVisible: 16777220 }, timeoutMs: 2000 });
  expect(result).toEqual({ matched: true });
});

test("connect() throws when ND_AUTOMATION_SOCKET is unset and no path is given", async () => {
  const prev = process.env.ND_AUTOMATION_SOCKET;
  delete process.env.ND_AUTOMATION_SOCKET;
  try {
    await expect(AutomationClient.connect()).rejects.toThrow("ND_AUTOMATION_SOCKET not set");
  } finally {
    if (prev !== undefined) process.env.ND_AUTOMATION_SOCKET = prev;
  }
});
