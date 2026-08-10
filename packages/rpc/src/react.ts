// React binding, at its own entry point (`@nativedesktop/rpc/react`) so the
// core client stays free of a React dependency. Hooks come from
// @nativedesktop/react, never react directly (dev-react.ts's
// pinned-dispatcher contract for `nd dev` hot re-eval).

import { useMemo, useSyncExternalStore } from "@nativedesktop/react";
import type { RpcClient, RpcContract, RpcState } from "./client.ts";

export interface RpcStatus {
  state: RpcState;
  attempt: number;
  nextRetryInMs: number | undefined;
  detail: string | undefined;
}

/** Subscribes a component to the client's connection status. The snapshot is
 * rebuilt inside the state-change callback, where `attempt`/`nextRetryInMs`
 * are guaranteed to already reflect the transition being notified. */
export function useRpcStatus<C extends RpcContract>(client: RpcClient<C>): RpcStatus {
  const source = useMemo(() => {
    let snapshot: RpcStatus = {
      state: client.state,
      attempt: client.attempt,
      nextRetryInMs: client.nextRetryInMs,
      detail: undefined,
    };
    return {
      subscribe: (onChange: () => void) =>
        client.onStateChange((state, detail) => {
          snapshot = { state, attempt: client.attempt, nextRetryInMs: client.nextRetryInMs, detail };
          onChange();
        }),
      get: () => snapshot,
    };
  }, [client]);
  return useSyncExternalStore(source.subscribe, source.get);
}
