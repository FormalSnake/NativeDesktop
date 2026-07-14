// Wire types shared by the main-thread client (client.ts) and the SQLite
// worker (sqlite.worker.ts). Every value here crosses a `postMessage`
// boundary, so it must be structured-clone-safe: only SQLite-representable
// scalars, no functions, no class instances. That constraint is why a
// transaction is a *list of statements* (TxStep[]) and not a callback — a
// closure can't be cloned to the worker.

/** A single bindable SQLite value. `bigint`/`Uint8Array` clone fine over postMessage. */
export type SqlValue = string | number | bigint | boolean | null | Uint8Array;

/** Positional bindings (`?`, `?1`) or named bindings (`$id`, `:id`, `@id`). */
export type SqlParams = readonly SqlValue[] | Readonly<Record<string, SqlValue>>;

/** Result of a mutation, mirroring bun:sqlite's `Statement.run()` return. */
export interface RunResult {
  changes: number;
  lastInsertRowid: number | bigint;
}

/** One statement in an atomic transaction batch. */
export interface TxStep {
  sql: string;
  params?: SqlParams;
}

/** Passed straight through to `new Database(filename, options)` in the worker. */
export interface OpenOptions {
  readonly?: boolean;
  create?: boolean;
  readwrite?: boolean;
}

export type WorkerRequest =
  | { id: number; kind: "open"; filename: string; options?: OpenOptions }
  | { id: number; kind: "query"; sql: string; params?: SqlParams }
  | { id: number; kind: "mutate"; sql: string; params?: SqlParams }
  | { id: number; kind: "transaction"; steps: readonly TxStep[] }
  | { id: number; kind: "close" };

export interface SerializedError {
  message: string;
  name: string;
  code?: string;
}

export type WorkerResponse =
  | { id: number; ok: true; result: unknown }
  | { id: number; ok: false; error: SerializedError };
