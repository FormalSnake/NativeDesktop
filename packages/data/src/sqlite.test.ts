// Runtime verification for the worker-backed SQLite client. Run with:
//   bun test packages/data/src/sqlite.test.ts
//
// Proves both halves of the promise: queries work end-to-end across the worker
// boundary (round-trip, named params, transactions, error propagation), and a
// deliberately slow query never blocks the Bun main thread that drives React.

import { afterAll, expect, test } from "bun:test";
import { openDatabase, type SqliteDatabase } from "./index.ts";

let db: SqliteDatabase;

afterAll(async () => {
  await db?.close();
});

test("query + mutate round-trip through the worker", async () => {
  db = await openDatabase(":memory:");
  await db.mutate("CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT, pinned INTEGER)");

  const inserted = await db.mutate("INSERT INTO notes (title, pinned) VALUES (?, ?)", ["first", true]);
  expect(inserted.changes).toBe(1);
  expect(inserted.lastInsertRowid).toBe(1);

  const rows = await db.query<{ id: number; title: string; pinned: number }>("SELECT * FROM notes ORDER BY id");
  expect(rows).toEqual([{ id: 1, title: "first", pinned: 1 }]);

  const named = await db.query("SELECT title FROM notes WHERE id = $id", { $id: 1 });
  expect(named).toEqual([{ title: "first" }]);
});

test("transaction commits a batch atomically", async () => {
  await db.mutate("DELETE FROM notes");
  const results = await db.transaction([
    { sql: "INSERT INTO notes (title, pinned) VALUES (?, 0)", params: ["a"] },
    { sql: "INSERT INTO notes (title, pinned) VALUES (?, 1)", params: ["b"] },
  ]);
  expect(results.map((r) => r.changes)).toEqual([1, 1]);
  const rows = await db.query<{ c: number }>("SELECT count(*) AS c FROM notes");
  expect(rows[0]?.c).toBe(2);
});

test("a failing transaction rolls back and rejects", async () => {
  await db.mutate("DELETE FROM notes");
  await expect(
    db.transaction([
      { sql: "INSERT INTO notes (title, pinned) VALUES (?, 0)", params: ["kept?"] },
      { sql: "INSERT INTO notes (nonexistent) VALUES (1)" }, // throws in the worker
    ]),
  ).rejects.toThrow(/nonexistent/);
  const rows = await db.query<{ c: number }>("SELECT count(*) AS c FROM notes");
  expect(rows[0]?.c).toBe(0); // the first insert was rolled back
});

test("a query error rejects with a usable message", async () => {
  await expect(db.query("SELECT * FROM does_not_exist")).rejects.toThrow(/no such table: does_not_exist/);
});

test("a slow query does not block the main thread", async () => {
  let ticks = 0;
  const heartbeat = setInterval(() => ticks++, 10);
  const order: string[] = [];

  const started = performance.now();
  const slowSql =
    "WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < 3000000) SELECT count(*) AS c FROM c";
  const slow = db.query<{ c: number }>(slowSql).then((rows) => {
    order.push("slow-query");
    return rows[0]?.c;
  });

  // An independent main-thread timer far shorter than the query. If the main
  // thread were blocked by the query, this 50ms timer could not fire first.
  const independent = new Promise<void>((resolve) => setTimeout(resolve, 50)).then(() => {
    order.push("independent-timer");
  });

  const [count] = await Promise.all([slow, independent]);
  clearInterval(heartbeat);
  const elapsed = performance.now() - started;

  console.log(
    `slow query returned count=${count} after ${elapsed.toFixed(0)}ms; ` +
      `main thread ticked ${ticks} times and the 50ms timer resolved first (order: ${order.join(" -> ")})`,
  );

  expect(count).toBe(3000000);
  expect(elapsed).toBeGreaterThan(100); // the query really was slow
  expect(order[0]).toBe("independent-timer"); // timer beat the query -> not blocked
  expect(ticks).toBeGreaterThan(3); // heartbeat kept advancing during the query
});
