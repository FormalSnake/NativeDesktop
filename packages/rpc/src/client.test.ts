// Client state-machine tests against the fake transport. Each test pins one
// of the comment-encoded ladder invariants in client.ts; timings use small
// real timers with wide margins rather than a mocked scheduler.

import { describe, expect, test } from "bun:test";
import { RpcClient } from "./client.ts";
import type { RpcClientOptions, RpcContract, RpcState } from "./client.ts";
import { fakeTransport } from "./fake-transport.ts";
import type { FakeTransport } from "./fake-transport.ts";

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

function makeClient(ft: FakeTransport, over: Partial<RpcClientOptions<RpcContract>> = {}): RpcClient<RpcContract> {
  return new RpcClient<RpcContract>({
    transport: ft.factory,
    handshake: { method: "hello", params: { token: "t" } },
    connectTimeoutMs: 40,
    callTimeoutMs: 500,
    backoff: { baseMs: 30, jitter: 0 },
    ...over,
  });
}

/** Dials, opens the latest conn, answers its handshake, awaits readiness. */
async function connectReady(
  ft: FakeTransport,
  client: RpcClient<RpcContract>,
  opts?: { retryForever?: boolean },
): Promise<void> {
  const p = client.connect(opts);
  const conn = ft.latest();
  conn.open();
  conn.reply(conn.callFor("hello")!.id!, { ok: true });
  await p;
}

describe("RpcClient", () => {
  test("connect timeout rejects the caller and still schedules the next rung", async () => {
    const ft = fakeTransport();
    const client = makeClient(ft);
    const p = client.connect({ retryForever: true });
    expect(ft.conns.length).toBe(1);
    await expect(p).rejects.toThrow(/timed out after 40ms/);
    expect(ft.conns[0]!.clientClosed).toBe(true);
    expect(client.state).toBe("reconnecting");
    expect(client.attempt).toBe(1);
    await sleep(60); // backoff base is 30ms
    expect(ft.conns.length).toBe(2);
    client.close();
  });

  test("a drop mid-handshake does not emit offline while retryForever", async () => {
    const ft = fakeTransport();
    const client = makeClient(ft);
    const states: RpcState[] = [];
    client.onStateChange((s) => states.push(s));
    const p = client.connect({ retryForever: true });
    ft.latest().open();
    expect(ft.latest().callFor("hello")).toBeDefined();
    ft.latest().drop();
    await expect(p).rejects.toThrow("connection closed before authentication");
    expect(states).not.toContain("offline");
    expect(client.state).toBe("reconnecting");
    client.close();
  });

  test("a queued call survives one reconnect and lands after the handshake", async () => {
    const ft = fakeTransport();
    const client = makeClient(ft);
    await connectReady(ft, client);
    ft.latest().drop();
    expect(client.state).toBe("reconnecting");
    const callP = client.call("doThing", { x: 1 });
    await sleep(60); // next rung dials at 30ms
    expect(ft.conns.length).toBe(2);
    const conn = ft.latest();
    conn.open();
    // Nothing but the handshake goes out until it answers.
    expect(conn.callFor("doThing")).toBeUndefined();
    conn.reply(conn.callFor("hello")!.id!, {});
    const flushed = conn.callFor("doThing");
    expect(flushed).toBeDefined();
    expect(flushed!.params).toEqual({ x: 1 });
    conn.reply(flushed!.id!, "done");
    expect(await callP).toBe("done");
    client.close();
  });

  test("an in-flight call rejects on drop", async () => {
    const ft = fakeTransport();
    const client = makeClient(ft);
    await connectReady(ft, client);
    const conn = ft.latest();
    const p = client.call("doThing", {});
    expect(conn.callFor("doThing")).toBeDefined();
    conn.drop();
    await expect(p).rejects.toThrow("connection closed");
    client.close();
  });

  test("the call deadline fires while queued on a link that stays down", async () => {
    const ft = fakeTransport();
    // Backoff far past the deadline: the link genuinely stays down.
    const client = makeClient(ft, { callTimeoutMs: 60, backoff: { baseMs: 10_000, jitter: 0 } });
    await connectReady(ft, client);
    ft.latest().drop();
    const p = client.call("doThing", {});
    await expect(p).rejects.toThrow(/doThing timed out after 60ms/);
    client.close();
  });

  test("a fatal handshake error suppresses reconnect and leaves the descriptive state", async () => {
    const ft = fakeTransport();
    const client = makeClient(ft, {
      handshake: { method: "hello", params: {}, fatal: (err) => err.rpcCode === 401 },
    });
    const seen: Array<{ state: RpcState; detail?: string }> = [];
    client.onStateChange((state, detail) => seen.push({ state, detail }));
    const p = client.connect({ retryForever: true });
    const conn = ft.latest();
    conn.open();
    conn.replyError(conn.callFor("hello")!.id!, 401, "bad token");
    await expect(p).rejects.toThrow("bad token");
    expect(client.state).toBe("offline");
    expect(seen.at(-1)).toEqual({ state: "offline", detail: "bad token" });
    await sleep(80); // well past the 30ms rung that must NOT have been armed
    expect(ft.conns.length).toBe(1);
    client.close();
  });

  test("a non-fatal handshake error keeps riding the ladder", async () => {
    const ft = fakeTransport();
    const client = makeClient(ft, {
      handshake: { method: "hello", params: {}, fatal: (err) => err.rpcCode === 401 },
    });
    const p = client.connect({ retryForever: true });
    const conn = ft.latest();
    conn.open();
    conn.replyError(conn.callFor("hello")!.id!, 500, "busy");
    await expect(p).rejects.toThrow("busy");
    expect(client.state).toBe("reconnecting");
    await sleep(60);
    expect(ft.conns.length).toBe(2);
    client.close();
  });

  test("the watchdog probes at idleProbe and closes at idleClose", async () => {
    const ft = fakeTransport();
    const client = makeClient(ft, {
      watchdogIntervalMs: 15,
      idleProbeMs: 60,
      idleCloseMs: 150,
      probe: { method: "ping", params: {} },
    });
    await connectReady(ft, client);
    const conn = ft.latest();
    await sleep(100); // past idleProbe with a few ticks of margin
    expect(conn.callFor("ping")).toBeDefined();
    expect(conn.clientClosed).toBe(false);
    await sleep(120); // past idleClose (the probe was never answered)
    expect(conn.clientClosed).toBe(true);
    expect(client.state).toBe("reconnecting");
    client.close();
  });

  test("a wall-clock jump forces an immediate probe", async () => {
    const ft = fakeTransport();
    let offset = 0;
    const client = makeClient(ft, {
      watchdogIntervalMs: 20,
      idleProbeMs: 10_000,
      idleCloseMs: 60_000,
      probe: { method: "ping", params: {} },
      now: () => Date.now() + offset,
    });
    await connectReady(ft, client);
    const conn = ft.latest();
    await sleep(35); // at least one normal tick so lastTick is fresh
    expect(conn.callFor("ping")).toBeUndefined(); // silence budget far from due
    offset = 1000; // suspend/resume: > watchdogIntervalMs * 3 between ticks
    await sleep(35);
    expect(conn.callFor("ping")).toBeDefined();
    client.close();
  });

  test("connect() during a pending backoff leaks neither timer nor socket", async () => {
    const ft = fakeTransport();
    const client = makeClient(ft, { connectTimeoutMs: 30, backoff: { baseMs: 100, jitter: 0 } });
    const p1 = client.connect({ retryForever: true });
    await expect(p1).rejects.toThrow(/timed out/);
    expect(ft.conns.length).toBe(1); // rung armed for +100ms, not dialed yet
    const p2 = client.connect({ retryForever: true });
    expect(ft.conns.length).toBe(2); // fresh dial, immediately
    const conn = ft.latest();
    conn.open();
    conn.reply(conn.callFor("hello")!.id!, { ok: true });
    await p2;
    expect(client.state).toBe("ready");
    expect(ft.conns[0]!.clientClosed).toBe(true);
    await sleep(150); // past the superseded backoff timer
    expect(ft.conns.length).toBe(2); // it was cleared, no ghost dial
    client.close();
  });

  test("nextRetryInMs read from inside the state callback equals the armed delay", async () => {
    const ft = fakeTransport();
    const client = makeClient(ft, { connectTimeoutMs: 40, backoff: { baseMs: 50, jitter: 0 } });
    const rungs: Array<{ attempt: number; next: number | undefined }> = [];
    client.onStateChange((s) => {
      if (s === "reconnecting" && client.nextRetryInMs !== undefined) {
        rungs.push({ attempt: client.attempt, next: client.nextRetryInMs });
      }
    });
    await connectReady(ft, client);
    ft.latest().drop();
    // Rung 1 armed synchronously inside the drop's close cascade.
    expect(rungs).toEqual([{ attempt: 1, next: 50 }]);
    await sleep(70); // rung 1 dials at 50ms; that dial times out at +40ms
    await sleep(60);
    expect(rungs).toEqual([
      { attempt: 1, next: 50 },
      { attempt: 2, next: 100 },
    ]);
    client.close();
  });

  test("events dispatch to on() subscribers and refresh the rx clock", async () => {
    const ft = fakeTransport();
    const client = makeClient(ft);
    await connectReady(ft, client);
    const got: unknown[] = [];
    const off = client.on("event.tick", (p) => got.push(p));
    ft.latest().notify("event.tick", { n: 1 });
    expect(got).toEqual([{ n: 1 }]);
    off();
    ft.latest().notify("event.tick", { n: 2 });
    expect(got).toEqual([{ n: 1 }]);
    client.close();
  });

  test("call() fails fast only when there is no ladder to wait for", async () => {
    const ft = fakeTransport();
    const client = makeClient(ft);
    await expect(client.call("doThing", {})).rejects.toThrow("not connected");
    await connectReady(ft, client);
    client.close();
    await expect(client.call("doThing", {})).rejects.toThrow("not connected");
  });

  test("close() rejects pending calls and stops the machine", async () => {
    const ft = fakeTransport();
    const client = makeClient(ft);
    await connectReady(ft, client);
    const p = client.call("doThing", {});
    client.close();
    await expect(p).rejects.toThrow("client closed");
    expect(client.state).toBe("closed");
    await sleep(60);
    expect(ft.conns.length).toBe(1); // no ladder after an explicit close
  });

  test("handshakeResult exposes the latest handshake and resume runs on the first dial", async () => {
    const ft = fakeTransport();
    let resumes = 0;
    let reconnects = 0;
    const client = makeClient(ft, {
      resume: async () => {
        resumes += 1;
      },
    });
    client.onReconnected(() => (reconnects += 1));
    await connectReady(ft, client);
    await sleep(1); // resume is async void off the ready transition
    expect(client.handshakeResult).toEqual({ ok: true });
    expect(resumes).toBe(1);
    expect(reconnects).toBe(0); // first dial is not a reconnect
    ft.latest().drop();
    await sleep(60);
    const conn = ft.latest();
    conn.open();
    conn.reply(conn.callFor("hello")!.id!, { ok: 2 });
    await sleep(5);
    expect(client.handshakeResult).toEqual({ ok: 2 });
    expect(resumes).toBe(2);
    expect(reconnects).toBe(1);
    client.close();
  });
});
