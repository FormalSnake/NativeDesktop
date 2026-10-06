// Solid binding, at its own entry point (`@nativedesktop/rpc/solid`) so the
// core client stays free of a solid-js dependency.

import { createSignal, onCleanup, type Accessor } from "solid-js";
import type { RpcClient, RpcContract } from "./client.ts";
import { createRpcStatusStore } from "./status-store.ts";
import type { RpcStatus } from "./status-store.ts";

export type { RpcStatus } from "./status-store.ts";

/** The client's connection status as a signal, unsubscribed when the owner is disposed. */
export function createRpcStatus<C extends RpcContract>(client: RpcClient<C>): Accessor<RpcStatus> {
  const store = createRpcStatusStore(client);
  const [status, setStatus] = createSignal(store.get());
  onCleanup(store.subscribe(() => setStatus(store.get())));
  return status;
}
