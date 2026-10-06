// Optional Solid binding for @nativedesktop/data. Import from
// "@nativedesktop/data/solid". Kept separate from the core so apps that only
// want the async client never pull in solid-js.

import { createMemo, type Accessor } from "solid-js";
import type { SqliteDatabase } from "./client.ts";
import type { SqlParams } from "./protocol.ts";

type MaybeAccessor<T> = T | Accessor<T>;

const read = <T>(value: MaybeAccessor<T>): T => (typeof value === "function" ? (value as Accessor<T>)() : value);

/**
 * Runs a read query as an async memo: reading the result inside a
 * `<Loading>` boundary shows its fallback until the rows arrive, and a failed
 * query throws to the nearest `<Errored>`. `db`, `sql` and `params` may be
 * accessors; a change re-runs the query and Solid drops the superseded
 * result. A nullish `db` (still opening) keeps the query pending.
 */
export function createQuery<Row = Record<string, unknown>>(
  db: MaybeAccessor<SqliteDatabase | null | undefined>,
  sql: MaybeAccessor<string>,
  params?: MaybeAccessor<SqlParams | undefined>,
): Accessor<Row[]> {
  return createMemo(() => {
    // Every reactive read happens before the first await, or the memo would not track it.
    const conn = read(db);
    const text = read(sql);
    const bound = read(params);
    if (!conn) return new Promise<Row[]>(() => {});
    return conn.query<Row>(text, bound);
  });
}
