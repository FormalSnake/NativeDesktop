// Optional React binding for @nativedesktop/data. Import from
// "@nativedesktop/data/react". Kept separate from the core so apps that only
// want the async client never pull in React.

import { useEffect, useState } from "@nativedesktop/react";
import type { SqliteDatabase } from "./client.ts";
import type { SqlParams } from "./protocol.ts";

export interface QueryState<Row> {
  data: Row[] | undefined;
  error: Error | undefined;
  loading: boolean;
}

/**
 * Run a read query and track its result as it resolves. Re-runs when `db`,
 * `sql`, or `params` change; a superseded or unmounted query is ignored so
 * late replies can't overwrite fresh state. Pass a nullish `db` (e.g. while it
 * is still opening) to stay in the loading state without querying.
 */
export function useQuery<Row = Record<string, unknown>>(
  db: SqliteDatabase | null | undefined,
  sql: string,
  params?: SqlParams,
): QueryState<Row> {
  const [state, setState] = useState<QueryState<Row>>({ data: undefined, error: undefined, loading: true });
  const key = JSON.stringify([sql, params]);

  // effect:audited — subscribes to an out-of-tree async source (the worker);
  // re-keyed on sql/params so a changed query refetches with fresh bindings.
  useEffect(() => {
    if (!db) return;
    let cancelled = false;
    setState((prev) => (prev.loading ? prev : { ...prev, loading: true }));
    db.query<Row>(sql, params).then(
      (rows) => {
        if (!cancelled) setState({ data: rows, error: undefined, loading: false });
      },
      (err: unknown) => {
        if (!cancelled) setState({ data: undefined, error: err as Error, loading: false });
      },
    );
    return () => {
      cancelled = true;
    };
    // params is captured through `key`; listing it directly would re-run on every render.
  }, [db, key]);

  return state;
}
