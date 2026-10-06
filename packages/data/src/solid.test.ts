import { afterAll, beforeAll, expect, test } from "bun:test";
import { createEffect, createRoot, createSignal, flush } from "solid-js";
import { openDatabase, type SqliteDatabase } from "./index.ts";
import { createQuery } from "./solid.ts";

let db: SqliteDatabase;

beforeAll(async () => {
  db = await openDatabase(":memory:");
  await db.mutate("create table t (n integer)");
  await db.mutate("insert into t values (1), (2), (3)");
});

afterAll(() => db.close());

async function until(cond: () => boolean): Promise<void> {
  for (let i = 0; i < 200 && !cond(); i++) {
    flush();
    await new Promise((r) => setTimeout(r, 5));
  }
  expect(cond()).toBe(true);
}

test("createQuery resolves rows and re-runs when a reactive param changes", async () => {
  const seen: number[][] = [];
  const [min, setMin] = createSignal(1);
  const dispose = createRoot((dispose) => {
    const rows = createQuery<{ n: number }>(db, "select n from t where n >= ?1 order by n", () => [min()]);
    createEffect(rows, (r) => {
      seen.push(r.map((row) => row.n));
    });
    return dispose;
  });
  await until(() => seen.length === 1);
  expect(seen[0]).toEqual([1, 2, 3]);

  setMin(3);
  await until(() => seen.length === 2);
  expect(seen[1]).toEqual([3]);
  dispose();
});

test("createQuery stays pending while the database is still opening", async () => {
  const seen: unknown[] = [];
  const [conn, setConn] = createSignal<SqliteDatabase | undefined>(undefined);
  const dispose = createRoot((dispose) => {
    const rows = createQuery(conn, "select count(*) as c from t");
    createEffect(rows, (r) => {
      seen.push(r);
    });
    return dispose;
  });
  await new Promise((r) => setTimeout(r, 30));
  flush();
  expect(seen).toEqual([]);

  setConn(db);
  await until(() => seen.length === 1);
  expect(seen[0]).toEqual([{ c: 3 }]);
  dispose();
});
