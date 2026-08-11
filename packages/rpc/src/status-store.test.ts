// Pins the subscribe-time resync: a transition landing between the store's
// creation (render) and subscribe (passive effect) fires into an empty
// handler set, and subscribe must rebuild the snapshot so the
// useSyncExternalStore post-subscribe re-read can observe it.

import { describe, expect, test } from "bun:test";
import { RpcClient } from "./client.ts";
import type { RpcContract } from "./client.ts";
import { fakeTransport } from "./fake-transport.ts";
import { createRpcStatusStore } from "./status-store.ts";

describe("createRpcStatusStore", () => {
  test("subscribe resyncs a transition that fired before any subscriber", async () => {
    const ft = fakeTransport();
    const client = new RpcClient<RpcContract>({
      transport: ft.factory,
      handshake: { method: "hello", params: {} },
    });
    const p = client.connect();
    const store = createRpcStatusStore(client); // "renders" while connecting
    expect(store.get().state).toBe("connecting");
    const conn = ft.latest();
    conn.open();
    conn.reply(conn.callFor("hello")!.id!, { ok: true });
    await p; // ready now, with nothing subscribed
    let notified = 0;
    const off = store.subscribe(() => (notified += 1));
    expect(store.get().state).toBe("ready");
    expect(notified).toBe(1);
    off();
    client.close();
  });

  test("subscribe with no missed transition leaves the snapshot alone", () => {
    const ft = fakeTransport();
    const client = new RpcClient<RpcContract>({ transport: ft.factory });
    const store = createRpcStatusStore(client);
    const before = store.get();
    let notified = 0;
    const off = store.subscribe(() => (notified += 1));
    expect(store.get()).toBe(before);
    expect(notified).toBe(0);
    off();
    client.close();
  });
});
