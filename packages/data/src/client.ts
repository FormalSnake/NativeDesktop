// Main-thread client for the worker-backed SQLite connection. Nothing here
// touches `bun:sqlite`; every call posts a message to sqlite.worker.ts and
// returns a Promise resolved when the worker replies, so the Bun main thread
// (which also runs React's commit loop) never blocks on a query.

import type {
  OpenOptions,
  RunResult,
  SerializedError,
  SqlParams,
  TxStep,
  WorkerRequest,
  WorkerResponse,
} from "./protocol.ts";

interface PendingCall {
  resolve: (value: unknown) => void;
  reject: (reason: Error) => void;
}

// A plain `Omit<WorkerRequest, "id">` collapses the discriminated union to its
// shared `kind` key; distributing over the union keeps each variant's fields.
type DistributiveOmit<T, K extends PropertyKey> = T extends unknown ? Omit<T, K> : never;
type RequestBody = DistributiveOmit<WorkerRequest, "id">;

function rebuildError(error: SerializedError): Error {
  const err = new Error(error.message);
  err.name = error.name;
  if (error.code !== undefined) (err as { code?: string }).code = error.code;
  return err;
}

/**
 * The async execution contract every call in this package ultimately speaks:
 * `sql` + structured-clone-safe `params` in, a Promise of results out, with the
 * synchronous `bun:sqlite` work happening in the worker. `SqliteDatabase`
 * implements it; `openDatabase()` hands you one.
 *
 * This is the stable extension seam for query builders / ORMs. An adapter is a
 * userland function that takes a `SqliteExecutor` and drives its own tool
 * against these three methods — so the framework never depends on any ORM, and
 * the app owns the ORM (its dependency, its version). Two example adapters,
 * `drizzle-orm/sqlite-proxy` and a custom Kysely `Dialect`, are proven in
 * `src/adapters.test.ts`; each is <20 lines against this interface.
 */
export interface SqliteExecutor {
  /** Run a read query. Returns every row as a plain object keyed by column name. */
  query<Row = Record<string, unknown>>(sql: string, params?: SqlParams): Promise<Row[]>;
  /** Run an INSERT/UPDATE/DELETE (or any exec). Returns the affected-row count and last insert rowid. */
  mutate(sql: string, params?: SqlParams): Promise<RunResult>;
  /** Run a batch of statements atomically (BEGIN/COMMIT, rolled back if any step throws). */
  transaction(steps: readonly TxStep[]): Promise<RunResult[]>;
}

export class SqliteDatabase implements SqliteExecutor {
  readonly #worker: Worker;
  #seq = 0;
  #closed = false;
  readonly #pending = new Map<number, PendingCall>();

  private constructor(worker: Worker) {
    this.#worker = worker;
    worker.addEventListener("message", (event: MessageEvent<WorkerResponse>) => this.#onMessage(event.data));
    // A worker-level error (e.g. the worker module failing to load) can never
    // be tied to one request, so fail every in-flight call rather than hang.
    worker.addEventListener("error", (event: ErrorEvent) =>
      this.#failAll(new Error(`sqlite worker error: ${event.message}`)),
    );
  }

  static async open(filename: string, options?: OpenOptions): Promise<SqliteDatabase> {
    const worker = new Worker(new URL("./sqlite.worker.ts", import.meta.url).href, { type: "module" });
    const db = new SqliteDatabase(worker);
    try {
      await db.#send({ kind: "open", filename, options });
    } catch (err) {
      worker.terminate();
      throw err;
    }
    return db;
  }

  /** Run a read query. Returns every row as a plain object. */
  query<Row = Record<string, unknown>>(sql: string, params?: SqlParams): Promise<Row[]> {
    return this.#send<Row[]>({ kind: "query", sql, params });
  }

  /** Run an INSERT/UPDATE/DELETE (or any exec). Returns the affected-row count and last insert rowid. */
  mutate(sql: string, params?: SqlParams): Promise<RunResult> {
    return this.#send<RunResult>({ kind: "mutate", sql, params });
  }

  /** Run a batch of statements atomically (BEGIN/COMMIT, rolled back if any step throws). */
  transaction(steps: readonly TxStep[]): Promise<RunResult[]> {
    return this.#send<RunResult[]>({ kind: "transaction", steps });
  }

  /** Close the underlying database and terminate the worker. Idempotent. */
  async close(): Promise<void> {
    if (this.#closed) return;
    const closed = this.#send({ kind: "close" }); // send while still open
    this.#closed = true; // then reject any further calls
    try {
      await closed;
    } finally {
      this.#worker.terminate();
      this.#failAll(new Error("database is closed"));
    }
  }

  #send<T>(req: RequestBody): Promise<T> {
    if (this.#closed) return Promise.reject(new Error("database is closed"));
    const id = ++this.#seq;
    return new Promise<T>((resolve, reject) => {
      this.#pending.set(id, { resolve: resolve as (value: unknown) => void, reject });
      this.#worker.postMessage({ ...req, id } as WorkerRequest);
    });
  }

  #onMessage(res: WorkerResponse): void {
    const call = this.#pending.get(res.id);
    if (!call) return;
    this.#pending.delete(res.id);
    if (res.ok) call.resolve(res.result);
    else call.reject(rebuildError(res.error));
  }

  #failAll(err: Error): void {
    for (const call of this.#pending.values()) call.reject(err);
    this.#pending.clear();
  }
}

/**
 * Open a worker-backed SQLite database. `filename` is caller-provided — pass
 * `":memory:"`, an absolute path, or e.g. `` `${appDataDir()}/app.sqlite` ``.
 */
export function openDatabase(filename: string, options?: OpenOptions): Promise<SqliteDatabase> {
  return SqliteDatabase.open(filename, options);
}
