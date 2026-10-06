// @nativedesktop/data — a worker-backed, Promise-based SQLite data layer.
// The connection lives in a Bun Worker (sqlite.worker.ts) so heavy queries run
// off the main thread and never stall the renderer.
//
// The optional `createQuery` Solid binding lives at "@nativedesktop/data/solid"
// so this core entry stays free of any solid-js dependency.

export { openDatabase, SqliteDatabase } from "./client.ts";
export type { SqliteExecutor } from "./client.ts";
export type { OpenOptions, RunResult, SqlParams, SqlValue, TxStep } from "./protocol.ts";
