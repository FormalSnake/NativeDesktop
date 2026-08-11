// React binding, at its own entry point (`@nativedesktop/rpc/react`) so the
// core client stays free of a React dependency. Hooks come from
// @nativedesktop/react, never react directly (dev-react.ts's
// pinned-dispatcher contract for `nd dev` hot re-eval).

import { useMemo, useSyncExternalStore } from "@nativedesktop/react";
import type { RpcClient, RpcContract } from "./client.ts";
import { createRpcStatusStore } from "./status-store.ts";
import type { RpcStatus } from "./status-store.ts";

export type { RpcStatus } from "./status-store.ts";

/** Subscribes a component to the client's connection status. */
export function useRpcStatus<C extends RpcContract>(client: RpcClient<C>): RpcStatus {
  const source = useMemo(() => createRpcStatusStore(client), [client]);
  return useSyncExternalStore(source.subscribe, source.get);
}
