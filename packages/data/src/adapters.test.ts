// Worker-backed Drizzle and Kysely, proven end-to-end.
//
// Neither ORM ships in @nativedesktop/data — both are devDependencies of this
// package and appear ONLY as the userland adapter examples below. Each adapter
// is a small function written against the package's public `SqliteExecutor`
// contract (openDatabase() returns one that implements it); copy either into an
// app, install the ORM as the app's own dependency, and you get a worker-backed
// query builder whose heavy queries never touch React's commit loop.
//
// Run with: bun test packages/data/src/adapters.test.ts

import { afterAll, expect, test } from "bun:test";
import { eq, sql as dsql } from "drizzle-orm";
import { integer, sqliteTable, text } from "drizzle-orm/sqlite-core";
import { drizzle } from "drizzle-orm/sqlite-proxy";
import {
  CompiledQuery,
  type DatabaseConnection,
  type Dialect,
  type Driver,
  type Generated,
  Kysely,
  type QueryResult,
  SqliteAdapter,
  SqliteIntrospector,
  SqliteQueryCompiler,
} from "kysely";
import { openDatabase, type SqliteDatabase, type SqliteExecutor, type SqlParams } from "./index.ts";

// ── Example adapter: Drizzle via drizzle-orm/sqlite-proxy ────────────────────
// sqlite-proxy is Drizzle's official *async remote* driver: you hand it a
// callback and it awaits your rows, so the whole builder returns Promises even
// though Drizzle's bun-sqlite dialect is synchronous. That is what makes the
// transparent async path feasible without forking Drizzle — the sync dialect
// stays in the worker, only this proxy runs on the main thread. sqlite-proxy
// reconstructs each result from a POSITIONAL value array, so we re-key each
// object row bun:sqlite gives us into `Object.values` in projected-column order.
// Caveat: a join selecting two same-named columns collapses in an object — such
// selects need column aliases (or a `.values()`-shaped executor).
function drizzleOverWorker<TSchema extends Record<string, unknown>>(exec: SqliteExecutor, schema: TSchema) {
  return drizzle(
    async (sql, params, method) => {
      if (method === "run") {
        await exec.mutate(sql, params);
        return { rows: [] };
      }
      const rows = (await exec.query(sql, params)).map((row) => Object.values(row));
      return { rows: method === "get" ? (rows[0] as unknown[]) : rows };
    },
    { schema },
  );
}

// ── Example adapter: Kysely via a custom async Dialect ───────────────────────
// Kysely's driver model is async-first: DatabaseConnection.executeQuery returns
// a Promise of `{ rows }`, so it maps onto the worker with no shimming and no
// row-shape conversion (Kysely keys rows by column name, which is exactly what
// our `query` returns). We reuse Kysely's own SQLite compiler/adapter/
// introspector and supply only the driver.
class WorkerConnection implements DatabaseConnection {
  constructor(private readonly exec: SqliteExecutor) {}

  async executeQuery<R>(compiled: CompiledQuery): Promise<QueryResult<R>> {
    const node = compiled.query as { kind: string; returning?: unknown };
    const write =
      node.kind === "InsertQueryNode" ||
      node.kind === "UpdateQueryNode" ||
      node.kind === "DeleteQueryNode" ||
      node.kind === "MergeQueryNode";
    if (write && node.returning == null) {
      const r = await this.exec.mutate(compiled.sql, compiled.parameters as SqlParams);
      return { rows: [], numAffectedRows: BigInt(r.changes), insertId: BigInt(r.lastInsertRowid) };
    }
    return { rows: await this.exec.query<R>(compiled.sql, compiled.parameters as SqlParams) };
  }

  async *streamQuery<R>(): AsyncIterableIterator<QueryResult<R>> {
    throw new Error("streaming is not supported by the worker-backed SQLite driver");
  }
}

function kyselyOverWorker<DB>(exec: SqliteExecutor): Kysely<DB> {
  const connection = new WorkerConnection(exec);
  const driver: Driver = {
    async init() {},
    async acquireConnection() {
      return connection;
    },
    async beginTransaction(conn) {
      await conn.executeQuery(CompiledQuery.raw("begin"));
    },
    async commitTransaction(conn) {
      await conn.executeQuery(CompiledQuery.raw("commit"));
    },
    async rollbackTransaction(conn) {
      await conn.executeQuery(CompiledQuery.raw("rollback"));
    },
    async releaseConnection() {},
    async destroy() {},
  };
  const dialect: Dialect = {
    createDriver: () => driver,
    createQueryCompiler: () => new SqliteQueryCompiler(),
    createAdapter: () => new SqliteAdapter(),
    createIntrospector: (db) => new SqliteIntrospector(db),
  };
  return new Kysely<DB>({ dialect });
}

// ── Verification ─────────────────────────────────────────────────────────────

const notes = sqliteTable("notes", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  title: text("title").notNull(),
  pinned: integer("pinned", { mode: "boolean" }).notNull(),
});

let db: SqliteDatabase;

afterAll(async () => {
  await db?.close();
});

test("Drizzle over the worker returns correctly-mapped rows via sqlite-proxy", async () => {
  db = await openDatabase(":memory:");
  await db.mutate(
    "CREATE TABLE notes (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, pinned INTEGER NOT NULL)",
  );
  const d = drizzleOverWorker(db, { notes });

  await d.insert(notes).values([
    { title: "first", pinned: true },
    { title: "second", pinned: false },
  ]);

  // Full-row select: proves positional mapping AND type decoding — `pinned` comes
  // back as a real boolean (integer 1/0 in SQLite), which only works if the value
  // array reached Drizzle's mapResultRow in the right column order.
  const rows = await d.select().from(notes).orderBy(notes.id);
  expect(rows).toEqual([
    { id: 1, title: "first", pinned: true },
    { id: 2, title: "second", pinned: false },
  ]);

  // Projected single-row select: exercises the `get` path and bound params.
  const one = await d.select({ title: notes.title }).from(notes).where(eq(notes.id, 2)).get();
  expect(one).toEqual({ title: "second" });
});

test("Kysely over the worker returns correctly-mapped rows via a custom async dialect", async () => {
  interface KDB {
    notes: { id: Generated<number>; title: string; pinned: number };
  }
  const k = kyselyOverWorker<KDB>(db);

  const inserted = await k
    .insertInto("notes")
    .values({ title: "third", pinned: 1 })
    .executeTakeFirstOrThrow();
  expect(inserted.numInsertedOrUpdatedRows).toBe(1n); // sourced from our numAffectedRows
  expect(inserted.insertId).toBe(3n); // sourced from lastInsertRowid

  const rows = await k.selectFrom("notes").select(["title", "pinned"]).orderBy("id").execute();
  expect(rows).toEqual([
    { title: "first", pinned: 1 },
    { title: "second", pinned: 0 },
    { title: "third", pinned: 1 },
  ]);
});

test("a heavy Drizzle query does not block the main thread", async () => {
  const d = drizzleOverWorker(db, {});

  let ticks = 0;
  const heartbeat = setInterval(() => ticks++, 10);
  const order: string[] = [];

  const started = performance.now();
  // Runs synchronously inside the worker (Drizzle's dialect is sync); the main
  // thread only awaits the postMessage reply, so it must stay responsive.
  const heavy = d
    .all<[number]>(
      dsql`WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < 3000000) SELECT count(*) AS c FROM c`,
    )
    .then((result) => {
      order.push("drizzle-query");
      return Number(result[0]?.[0]);
    });

  const independent = new Promise<void>((resolve) => setTimeout(resolve, 50)).then(() => {
    order.push("independent-timer");
  });

  const [count] = await Promise.all([heavy, independent]);
  clearInterval(heartbeat);
  const elapsed = performance.now() - started;

  console.log(
    `heavy Drizzle query returned count=${count} after ${elapsed.toFixed(0)}ms; ` +
      `main thread ticked ${ticks} times and the 50ms timer resolved first (order: ${order.join(" -> ")})`,
  );

  expect(count).toBe(3000000);
  expect(elapsed).toBeGreaterThan(100); // the query really was slow
  expect(order[0]).toBe("independent-timer"); // timer beat the ORM query -> not blocked
  expect(ticks).toBeGreaterThan(3); // heartbeat kept advancing during the query
});
