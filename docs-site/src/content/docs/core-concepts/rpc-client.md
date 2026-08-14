---
title: RPC Client
description: "@nativedesktop/rpc: a resilient JSON-RPC 2.0 client with a reconnect ladder, call queueing, and a liveness watchdog, for apps that talk to a daemon or remote server."
---

The Bun child is a full runtime, so an app that talks to a local daemon or a remote server just
opens a socket. `@nativedesktop/rpc` (`packages/rpc/`) handles what goes wrong afterwards: silent
half-open sockets, reconnect storms, calls fired during a blip, laptops waking from suspend. It is a
JSON-RPC 2.0 client with an exponential-backoff reconnect ladder, a handshake gate, call queueing
across drops, and an rx-silence watchdog. Zero runtime dependencies, and no React in the core entry.

## Setup

```ts
import { RpcClient, webSocketTransport } from "@nativedesktop/rpc";

interface Contract {
  methods: {
    "notes.list": { params: {}; result: { id: string; title: string }[] };
    "host.status": { params: {}; result: { uptime: number } };
  };
  events: {
    "event.noteChanged": { id: string };
  };
}

const client = new RpcClient<Contract>({
  transport: webSocketTransport("ws://127.0.0.1:7050"),
  handshake: {
    method: "hello",
    params: () => ({ token: readToken() }), // a thunk re-reads per dial
    fatal: (err) => err.rpcCode === 401, // don't retry a rejected credential
  },
  resume: async (c) => {
    await c.call("events.subscribe", {}); // runs after EVERY successful handshake
  },
  probe: { method: "host.status", params: {} },
});

await client.connect(); // resolves with the handshake result
const notes = await client.call("notes.list", {});
const off = client.on("event.noteChanged", ({ id }) => refresh(id));
```

The contract type is purely compile-time: `call` and `on` are constrained by it, tRPC-style.
Wire format is JSON-RPC 2.0: the handshake (when configured) is the first frame on every socket
and nothing else is sent until it answers; requests are id-correlated; server pushes are
notifications (no id) dispatched to `on()` subscribers.

## Transports

A `TransportFactory` is called once per dial; constructing it starts the connect (WebSocket
semantics). Two are built in:

- `webSocketTransport(url, protocols?)`: the browser-global `WebSocket` (available in Bun).
- `socketTransport({ path } | { host, port })`: a raw unix/TCP socket with NDJSON framing (one
  JSON frame per newline).

Anything else (a child process's stdio, a mock in tests) implements the three-method seam:
`send`/`close` out, `onOpen`/`onMessage`/`onClose` in. `close()` must guarantee no handler fires
afterwards; the built-ins do.

## The reconnect ladder

Five rules govern failure handling:

- **Fail fast only before first success.** A first `connect()` against a dead endpoint rejects
  (`state: "offline"`), so an interactive flow can roll back a typo'd address. Once the handshake
  has succeeded at least once, or `connect({ retryForever: true })` was used (restored/persisted
  endpoints), every drop rides the ladder: 500ms, 1s, 2s, 4s, 8s, 16s, 16s, ... with +0..50%
  jitter, no attempt cap.
- **A stable connection resets the ladder.** 30s of connected time (`backoff.stabilityWindowMs`)
  means the next drop starts back at 500ms; a shorter span inherits the previous outage's rung.
- **Calls queue across a blip.** `call()` during a reconnect window queues and flushes after the
  next successful handshake instead of failing on millisecond timing. In-flight calls whose fate
  is unknown reject on the drop; every call is bounded by `callTimeoutMs` (default 15s) whether
  in flight or queued.
- **Silence is a verdict, probes are not.** While ready, the watchdog (5s cadence) sends `probe`
  at 20s of rx silence and force-closes the socket into the normal reconnect path at 40s: a
  wifi/VPN drop with no RST produces no close event, and this is what turns a silent dead socket
  into a reconnect. A wall-clock jump between ticks (laptop suspend) forces an immediate probe.
- **Fatal handshake errors stop the ladder.** When `handshake.fatal(err)` returns true (wrong
  protocol version, revoked token), the client stays `offline` with the server's message as
  detail instead of retrying a connect that can never succeed.

`reconnectNow()` re-dials immediately, bypassing a pending backoff wait; `close()` tears the
client down for good.

## Status in the UI

`client.state` is `"connecting" | "ready" | "reconnecting" | "offline" | "closed"`, with
`attempt` and `nextRetryInMs` exposing the ladder's numbers (readable synchronously from inside
an `onStateChange` callback; they never drift from what is actually scheduled).
`onReconnected(cb)` fires after a successful post-drop reconnect, the hook for re-hydrating
whatever the app missed while disconnected.

The React binding lives at its own entry point so the core stays React-free:

```tsx
import { useRpcStatus } from "@nativedesktop/rpc/react";

function ConnectionBadge(): React.ReactNode {
  const { state, attempt, nextRetryInMs } = useRpcStatus(client);
  if (state === "ready") return <label text="Connected" />;
  if (state === "reconnecting") {
    return <label text={`Reconnecting (attempt ${attempt}, next in ${Math.round(nextRetryInMs ?? 0) / 1000}s)`} />;
  }
  return <label text={state} />;
}
```

Server side, batching, and client-to-server notifications are out of scope: the client issues
id-correlated calls and receives notifications, nothing more.
