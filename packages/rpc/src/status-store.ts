// Snapshot store behind createRpcStatus, renderer-free so the subscribe-time
// resync is testable without a renderer. The snapshot is rebuilt inside the
// state-change callback, where `attempt`/`nextRetryInMs` are guaranteed to
// already reflect the transition being notified.

import type { RpcClient, RpcContract, RpcState } from "./client.ts";

export interface RpcStatus {
  state: RpcState;
  attempt: number;
  nextRetryInMs: number | undefined;
  detail: string | undefined;
}

export interface RpcStatusStore {
  subscribe(onChange: () => void): () => void;
  get(): RpcStatus;
}

export function createRpcStatusStore<C extends RpcContract>(client: RpcClient<C>): RpcStatusStore {
  let snapshot: RpcStatus = {
    state: client.state,
    attempt: client.attempt,
    nextRetryInMs: client.nextRetryInMs,
    detail: undefined,
  };
  return {
    subscribe: (onChange) => {
      const off = client.onStateChange((state, detail) => {
        snapshot = { state, attempt: client.attempt, nextRetryInMs: client.nextRetryInMs, detail };
        onChange();
      });
      // A transition between the store's creation and subscribe fired into
      // an empty handler set. Rebuild the snapshot and notify so the
      // subscriber's re-read observes it; returning the stale capture would
      // stick the UI on the old state forever. Its detail belonged to the
      // missed transition, so it resets.
      if (
        client.state !== snapshot.state ||
        client.attempt !== snapshot.attempt ||
        client.nextRetryInMs !== snapshot.nextRetryInMs
      ) {
        snapshot = {
          state: client.state,
          attempt: client.attempt,
          nextRetryInMs: client.nextRetryInMs,
          detail: undefined,
        };
        onChange();
      }
      return off;
    },
    get: () => snapshot,
  };
}
