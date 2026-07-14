// The SQLite worker: the ONLY place `bun:sqlite` is opened. It runs on its own
// thread, so every query here is off the Bun main thread that drives React's
// commit loop — a slow `SELECT` blocks this worker, never the UI.
//
// Requests arrive in order on the worker's event loop and are answered by `id`,
// so the client can have many in flight at once. The first request is always
// `open`; queries the client sends afterwards are guaranteed to see an open DB
// because openDatabase() awaits the open reply before handing back the client.

import { Database, type Statement } from "bun:sqlite";
import type { RunResult, SerializedError, SqlParams, TxStep, WorkerRequest, WorkerResponse } from "./protocol.ts";

declare const self: Worker;

let db: Database | null = null;

function reply(res: WorkerResponse): void {
  self.postMessage(res);
}

function serializeError(err: unknown): SerializedError {
  if (err instanceof Error) {
    const code = (err as { code?: unknown }).code;
    return { message: err.message, name: err.name, code: typeof code === "string" ? code : undefined };
  }
  return { message: String(err), name: "Error" };
}

// bun:sqlite accepts positional bindings as a spread and named bindings as a
// single object argument; normalize both shapes (and the no-params case) here.
function all(stmt: Statement, params?: SqlParams): unknown[] {
  if (params === undefined) return stmt.all() as unknown[];
  return Array.isArray(params) ? (stmt.all(...params) as unknown[]) : (stmt.all(params) as unknown[]);
}

function run(stmt: Statement, params?: SqlParams): RunResult {
  if (params === undefined) return stmt.run();
  return Array.isArray(params) ? stmt.run(...params) : stmt.run(params);
}

function requireDb(): Database {
  if (!db) throw new Error("database is not open");
  return db;
}

self.onmessage = (event: MessageEvent<WorkerRequest>): void => {
  const req = event.data;
  try {
    switch (req.kind) {
      case "open": {
        if (db) throw new Error("database is already open");
        db = new Database(req.filename, req.options);
        reply({ id: req.id, ok: true, result: null });
        break;
      }
      case "query": {
        const rows = all(requireDb().query(req.sql), req.params);
        reply({ id: req.id, ok: true, result: rows });
        break;
      }
      case "mutate": {
        const result = run(requireDb().query(req.sql), req.params);
        reply({ id: req.id, ok: true, result });
        break;
      }
      case "transaction": {
        const active = requireDb();
        // db.transaction() wraps the batch in BEGIN/COMMIT and rolls back +
        // rethrows if any step throws, giving us atomicity for free.
        const batch = active.transaction((steps: readonly TxStep[]): RunResult[] =>
          steps.map((step) => run(active.query(step.sql), step.params)),
        );
        reply({ id: req.id, ok: true, result: batch(req.steps) });
        break;
      }
      case "close": {
        db?.close();
        db = null;
        reply({ id: req.id, ok: true, result: null });
        break;
      }
    }
  } catch (err) {
    reply({ id: req.id, ok: false, error: serializeError(err) });
  }
};
