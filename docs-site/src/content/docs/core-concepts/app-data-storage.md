---
title: App Data & Storage
description: Where an app's persistent data lives on disk, and the worker-backed SQLite layer for querying it without blocking React's commit loop.
---

Every app gets a per-platform, per-app directory for persistent data, the same idea as Electron's
`app.getPath('userData')`, plus a SQLite layer that keeps disk I/O off the thread driving React's
commit loop.

## `getAppDataDir()` / `ensureAppDataDir()`

Both are exported from `@nativedesktop/react` (`packages/react/src/paths.ts`):

```tsx
import { ensureAppDataDir, getAppDataDir } from "@nativedesktop/react";

getAppDataDir(); // resolve the path; does not create it
ensureAppDataDir(); // resolve AND mkdir -p it, returning the same path
```

In a packaged app the name comes from the bundle's `nd-app.json` (written by
[`nd package`](/packaging/) from `app.name`); in dev it comes from the nearest `package.json`'s
`name` field (the same `cwd` `loadConfig()` resolves `nativedesktop.config.ts` from), so apps that
configure no `app.name` keep the same directory in both modes. The resolved path follows each OS's
own convention:

| Platform | Path |
| --- | --- |
| macOS | `~/Library/Application Support/<name>` |
| Linux | `$XDG_DATA_HOME/<name>` (falls back to `~/.local/share/<name>`) |
| Windows | `%APPDATA%/<name>` (backend not yet implemented; see [Platform Support](/native-platform/platform-support/)) |

`getAppDataDir()` just resolves the path; `ensureAppDataDir()` also creates it (`mkdirSync` with
`recursive: true`) and hands back the same string, so it's the one you want before writing a file or
opening a database there.

## `createStore`: versioned JSON settings

For settings, layouts, and other small persistent state, `createStore` (exported from
`@nativedesktop/react`, `packages/react/src/store.ts`) manages one `${name}.json` file under
`getAppDataDir()` (or a `dir` override):

```tsx
import { createStore, render, useStoreValue } from "@nativedesktop/react";

const settings = createStore<{ theme: string }>({
  name: "settings", // -> <appDataDir>/settings.json
  version: 1,
  defaults: { theme: "system" },
  migrate: (raw, fromVersion) => {
    // Called on EVERY load, current-version files included: this one hook is
    // where validation, sanitizing, and upgrades all live. Return null to
    // reject the file and start over from defaults.
    if (typeof raw !== "object" || raw === null) return null;
    return raw as { theme: string };
  },
});

await settings.load(); // top-level await, right before render()
await render(<App />);
```

Load before render. The app entry is already `await render(<App />)`, so `await store.load()` on the
line above makes `store.get()` synchronous inside every component: no loading flash, no restore
effect, no Suspense boundary. A `get()` before `load()` resolves throws a named error rather than
returning a silent default.

Inside components, subscribe with `useStoreValue(store)` (optionally `useStoreValue(store, select)`),
and write with `store.set(next)` or `store.update(fn)`. The API:

```ts
createStore<T>(options: StoreOptions<T>): Store<T> // deduped by resolved file path

load(): Promise<T>        // idempotent; repeat calls return the same promise
get(): T                  // synchronous after load()
set(next: T): void        // notifies subscribers, schedules a debounced write
update(fn: (prev: T) => T): void
subscribe(cb: (value: T) => void): () => void
flush(): Promise<void>    // awaits the pending debounced write plus every queued one
loadError: Error | undefined
```

Persistence semantics:

- **Debounced, atomic writes.** Sets are collapsed on a 250ms debounce (`debounceMs`), each write
  lands via write-to-tmp + rename, and writes are serialized so an older snapshot can never land
  after a newer one. Call `flush()` after a structural change you don't want to lose (a pane
  closed, a tab reordered); let ratio-drag-style churn ride the debounce.
- **Exit safety.** A last-resort synchronous flush of any pending value runs on `exit`, `SIGINT`,
  and `SIGTERM` (the host stops the Bun child with SIGTERM).
- **Versioning.** The file carries a `{ version, data }` envelope. `migrate(raw, fromVersion)` is
  the single validate-and-upgrade hook; without one, only exact-version files load.
- **Corruption.** An unparseable or rejected file is renamed to `${name}.corrupt.json` (one slot,
  overwritten), the store starts from `defaults`, and `loadError` records why: the app keeps
  launching, and the bad file stays rescuable.

Bring your own validator (zod and friends) inside `migrate`. The store depends on none.

## `@nativedesktop/data`: worker-backed SQLite

The Bun child is a full runtime rather than a sandboxed renderer (see
[Architecture](/core-concepts/architecture/)), so `bun:sqlite` is right there. It is also the same
thread that drives React's commit loop, where a slow query stalls UI updates.
`@nativedesktop/data` (`packages/data/`) runs the `bun:sqlite` connection inside a Bun `Worker`
(`packages/data/src/sqlite.worker.ts`) and exposes a Promise-based client on the main thread. Every
call is a `postMessage` round-trip, so a slow `SELECT` blocks the worker instead of your app.

```tsx
import { ensureAppDataDir } from "@nativedesktop/react";
import { openDatabase } from "@nativedesktop/data";

const db = await openDatabase(`${ensureAppDataDir()}/app.sqlite`);
// or openDatabase(":memory:") for a throwaway/test database

await db.mutate("CREATE TABLE IF NOT EXISTS notes (id INTEGER PRIMARY KEY, title TEXT)");
const inserted = await db.mutate("INSERT INTO notes (title) VALUES (?)", ["first"]);
// inserted: { changes: number; lastInsertRowid: number | bigint }

const rows = await db.query<{ id: number; title: string }>("SELECT * FROM notes ORDER BY id");

await db.transaction([
  { sql: "INSERT INTO notes (title) VALUES (?)", params: ["a"] },
  { sql: "INSERT INTO notes (title) VALUES (?)", params: ["b"] },
]); // BEGIN/COMMIT'd atomically; any step throwing rolls the whole batch back

await db.close(); // closes the database and terminates the worker
```

`SqliteDatabase` (`packages/data/src/client.ts`) exposes:

```ts
openDatabase(filename: string, options?: OpenOptions): Promise<SqliteDatabase>

query<Row = Record<string, unknown>>(sql: string, params?: SqlParams): Promise<Row[]>
mutate(sql: string, params?: SqlParams): Promise<RunResult>
transaction(steps: readonly TxStep[]): Promise<RunResult[]>
close(): Promise<void>
```

`filename` is caller-provided: an absolute path (composed with `ensureAppDataDir()` above),
`":memory:"`, or any path `bun:sqlite`'s `Database` constructor accepts; `options` (`readonly`,
`create`, `readwrite`) is passed straight through to it. `SqlParams` accepts either positional
bindings (`?`, `?1`, an array) or named bindings (`$id`/`:id`/`@id`, an object); both cross the
worker boundary as structured-clone-safe values (`string | number | bigint | boolean | null |
Uint8Array`). A `transaction` is a plain array of `{ sql, params? }` steps rather than a callback,
because a closure can't be cloned across `postMessage`.

### `useQuery`: the React binding

An optional hook lives at a separate entry point, `@nativedesktop/data/react`, so the core client
stays free of a React dependency for apps that don't want it:

```tsx
import { useQuery } from "@nativedesktop/data/react";

function NoteList({ db }: { db: SqliteDatabase | null }) {
  const { data, error, loading } = useQuery<{ id: number; title: string }>(db, "SELECT * FROM notes ORDER BY id");
  // re-runs when db, sql, or params change; a superseded or unmounted query is ignored
  // so a late reply can never clobber fresher state
  if (loading) return <label text="Loading…" />;
  if (error) return <label text={`Error: ${error.message}`} />;
  return <box>{data!.map((n) => <label key={n.id} text={n.title} />)}</box>;
}
```

Pass a nullish `db` (e.g. while `openDatabase()` is still resolving) to stay in the loading state
without querying.

### Bringing your own ORM

`@nativedesktop/data` depends on zero ORMs. `packages/data/package.json` has null `dependencies` and
`optionalDependencies`; `drizzle-orm` and `kysely` appear only under `devDependencies`, where the
package's adapter tests exercise them. Raw SQL through `query`/`mutate`/`transaction` stays
first-class.

The seam is `SqliteExecutor` (`packages/data/src/client.ts`, re-exported from `index.ts`), which is
the three async methods above minus `close`:

```ts
export interface SqliteExecutor {
  query<Row = Record<string, unknown>>(sql: string, params?: SqlParams): Promise<Row[]>;
  mutate(sql: string, params?: SqlParams): Promise<RunResult>;
  transaction(steps: readonly TxStep[]): Promise<RunResult[]>;
}
```

`SqliteDatabase` implements it, so anything written against `SqliteExecutor` works against a real
`openDatabase()` connection. An ORM adapter is a small userland function driving its query builder's
async driver hooks through these three methods. The app installs the ORM and owns its version. Two
adapters are proven end to end in `packages/data/src/adapters.test.ts`
(`bun test packages/data/src/adapters.test.ts`), including a test that a heavy query through the ORM
does not block the main thread.

**Drizzle** adapts via `drizzle-orm/sqlite-proxy`, its official async remote driver. Hand it a
callback and the whole query builder returns Promises, even though Drizzle's own bun-sqlite dialect
is synchronous. That is what makes an async worker-backed connection possible without forking
Drizzle:

```ts
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
```

sqlite-proxy reconstructs each row from a positional value array, so the adapter re-keys `query()`'s
named-column rows via `Object.values()` in projected-column order. Caveat: a join selecting two
same-named columns collapses under `Object.values()`, since a plain object cannot hold two keys with
the same name. Alias one of them in the SQL.

**Kysely** adapts via a custom `Dialect` and `Driver`. Its driver model is async from the start
(`DatabaseConnection.executeQuery` returns `Promise<{ rows }>`), so it maps onto `SqliteExecutor`
with no row-shape conversion: Kysely keys rows by column name, exactly what `query()` returns. Reuse
Kysely's own SQLite compiler, adapter, and introspector and supply only the driver:

```ts
class WorkerConnection implements DatabaseConnection {
  constructor(private readonly exec: SqliteExecutor) {}

  async executeQuery<R>(compiled: CompiledQuery): Promise<QueryResult<R>> {
    // INSERT/UPDATE/DELETE/MERGE without a RETURNING clause -> exec.mutate(),
    // mapped into { numAffectedRows, insertId }; everything else -> exec.query().
  }

  async *streamQuery<R>(): AsyncIterableIterator<QueryResult<R>> {
    throw new Error("streaming is not supported by the worker-backed SQLite driver");
  }
}
```

wired into a `Dialect` whose `createDriver()` returns a `Driver` that acquires a single
`WorkerConnection` and turns `beginTransaction`/`commitTransaction`/`rollbackTransaction` into plain
`BEGIN`/`COMMIT`/`ROLLBACK` statements through it.

### Migrations

Two ways to apply schema changes, both compatible with the worker-backed connection.

Use `drizzle-kit` unchanged. `drizzle-kit generate` and `drizzle-kit migrate` run as their own
process directly against the SQLite file, off the hot path, and neither knows nor cares that the
app's queries route through a worker.

Or apply them at startup, in-process, through `drizzle-orm/sqlite-proxy/migrator`:

```ts
import { migrate } from "drizzle-orm/sqlite-proxy/migrator";

await migrate(
  db,
  async (queries) => {
    await executor.transaction(queries.map((sql) => ({ sql })));
  },
  { migrationsFolder: "./drizzle" },
);
```

`queries` arrives as `string[]`. Mapping each one to `{ sql }` turns it into a `TxStep`, so the whole
migration runs as one `transaction()` call, atomic through the same worker every other query uses.
