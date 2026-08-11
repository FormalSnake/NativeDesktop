// Snapshot store behind useRpcStatus, React-free so the subscribe-time
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
      // A transition in the render-to-subscribe gap (a local handshake can
      // settle before passive effects flush) fired into an empty handler
      // set. Rebuild the snapshot so useSyncExternalStore's post-subscribe
      // getSnapshot() re-read observes it; returning the stale capture
      // would defeat that guard and stick the UI on the old state forever.
      // Its detail belonged to the missed transition, so it resets.
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
