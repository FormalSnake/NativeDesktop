import { expect, test } from "bun:test";
import { createRoot, flush } from "solid-js";
import { RpcClient, type RpcContract } from "./client.ts";
import { fakeTransport } from "./fake-transport.ts";
import { createRpcStatus } from "./solid.ts";

test("createRpcStatus follows the client and stops when its owner is disposed", async () => {
  const ft = fakeTransport();
  const client = new RpcClient<RpcContract>({ transport: ft.factory, handshake: { method: "hello", params: {} } });
  const p = client.connect();
  const { status, dispose } = createRoot((dispose) => ({ status: createRpcStatus(client), dispose }));
  expect(status().state).toBe("connecting");

  const conn = ft.latest();
  conn.open();
  conn.reply(conn.callFor("hello")!.id!, { ok: true });
  await p;
  flush();
  expect(status().state).toBe("ready");

  dispose();
  client.close();
  flush();
  expect(status().state).toBe("ready");
});
