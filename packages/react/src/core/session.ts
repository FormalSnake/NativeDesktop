// The one host connection a Bun child holds, shared by whichever renderer
// drives it. Kept on globalThis because `bun --hot` re-runs every module's
// top level on an edit: a module-local binding would reconnect, or lose the
// registry that routes events to live handlers.

import { lat } from "./lat.ts";
import { Ndp, type EventMsg } from "./ndp.ts";
import { Batch, NodeRegistry } from "./ops.ts";
import { currentGeneration } from "./ids.ts";
import { setBackend, setHostManifest } from "./platform.ts";
import { dispatchSystemEvent } from "./system.ts";

export interface Session {
  ndp: Ndp;
  registry: NodeRegistry;
  batch: Batch;
  /** Drains the batch into one CommitBatch frame; a no-op when it is empty. */
  commit(): void;
  /** Ships whatever the renderer has batched so far. A widget command goes
   *  out on its own frame, so it calls this first: a command sent in the same
   *  tick its target was created (Solid runs refs and onSettled before its
   *  batch commits) would otherwise reach the host before the create. A
   *  renderer that tracks per-batch state replaces it; the default is
   *  `commit`. */
  flush(): void;
}


declare global {
  // eslint-disable-next-line no-var
  var __nd_session: Session | undefined;
}

export function getSession(): Session | undefined {
  return globalThis.__nd_session;
}

export function setSession(s: Session | undefined): void {
  globalThis.__nd_session = s;
}

/** True under `nd dev` (ND_DEV=1), the only mode that runs `bun --hot`. */
export function isHot(): boolean {
  return process.env.ND_DEV === "1";
}

/** Connects and handshakes once per process; later calls (a hot re-eval)
 *  return the live session. */
export async function connect(): Promise<Session> {
  const existing = getSession();
  if (existing) return existing;
  const ndp = await Ndp.connect();
  // Registered BEFORE the handshake: the host replays the standing
  // app-activation and webview-engine state in systemEvents written just
  // ahead of the HelloAck, so they are dispatched before the handshake
  // resolves and the first render reads them.
  ndp.onSystemEvent((channel, data) => dispatchSystemEvent(channel, data));
  await ndp.handshake({ name: "bun", version: Bun.version });
  setBackend(ndp.backend);
  setHostManifest(ndp.hostWidgets, ndp.hostCommands);

  const batch = new Batch();
  const registry = new NodeRegistry();
  let commitId = 0;
  // ND_PERF_TRACE=1: `ND_PERF event` per host event (handler time) and
  // `ND_PERF commit` per commit (ops, time since the first event it answers).
  // `at` is wall-clock microseconds, the clock the host's lines use.
  const trace = process.env.ND_PERF_TRACE === "1";
  const at = () => Math.round((performance.timeOrigin + performance.now()) * 1000);
  let lastEvent = "";
  let lastEventAt = 0;
  const session: Session = {
    ndp,
    registry,
    batch,
    commit() {
      const ops = batch.drain();
      if (!ops.length) return;
      lat("js.commitSend", `commit=${commitId} ops=${ops.length}`);
      ndp.sendCommit({ commitId: commitId++, generation: currentGeneration(), ops });
      if (trace) {
        const t = at();
        console.error(`ND_PERF commit ops=${ops.length} after=${lastEvent || "-"} since_us=${lastEventAt ? t - lastEventAt : -1} at=${t}`);
        lastEvent = "";
        lastEventAt = 0;
      }
    },
    flush() {
      session.commit();
    },
  };
  ndp.onEvent((e: EventMsg) => {
    lat("js.event", `seq=${e.seq} node=${e.nodeId} name=${e.name}`);
    if (!trace) {
      registry.get(e.nodeId)?.handlers[e.name]?.(e.payload);
      lat("js.handled", `seq=${e.seq}`);
      return;
    }
    const t = at();
    registry.get(e.nodeId)?.handlers[e.name]?.(e.payload);
    lat("js.handled", `seq=${e.seq}`);
    if (!lastEventAt) {
      lastEvent = e.name;
      lastEventAt = t;
    }
    console.error(`ND_PERF event ${e.name} node=${e.nodeId} handler_us=${at() - t} at=${t}`);
  });
  setSession(session);
  return session;
}
