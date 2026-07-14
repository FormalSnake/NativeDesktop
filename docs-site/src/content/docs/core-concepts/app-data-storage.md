---
title: App Data & Storage
description: Where an app's persistent data lives on disk, and the worker-backed SQLite layer for querying it without blocking React's commit loop.
---

Every NativeDesktop app gets a per-platform, per-app directory for persistent data — the same idea
as Electron's `app.getPath('userData')` — plus a small SQLite layer that keeps disk I/O off the
thread driving React's commit loop.

## `getAppDataDir()` / `ensureAppDataDir()`

Both are exported from `@nativedesktop/react` (`packages/react/src/paths.ts`):

```tsx
import { ensureAppDataDir, getAppDataDir } from "@nativedesktop/react";

getAppDataDir(); // resolve the path; does not create it
ensureAppDataDir(); // resolve AND mkdir -p it, returning the same path
```

The app name comes from the nearest `package.json`'s `name` field (the same `cwd` `loadConfig()`
resolves `nativedesktop.config.ts` from) — there's no separate app-identity config yet. The
resolved path follows each OS's own convention:

| Platform | Path |
| --- | --- |
| macOS | `~/Library/Application Support/<name>` |
| Linux | `$XDG_DATA_HOME/<name>` (falls back to `~/.local/share/<name>`) |
| Windows | `%APPDATA%/<name>` (backend not yet implemented — see [Platform Support](/native-platform/platform-support/)) |

`getAppDataDir()` just resolves the path; `ensureAppDataDir()` also creates it (`mkdirSync` with
`recursive: true`) and hands back the same string, so it's the one you want before writing a file or
opening a database there.

## `@nativedesktop/data`: worker-backed SQLite

Recall from [Architecture](/core-concepts/architecture/) that the Bun child is a full runtime, not a
sandboxed renderer — `bun:sqlite` is right there. But it's still the same thread that drives React's
commit loop, so a slow query would stall UI updates. `@nativedesktop/data` (`packages/data/`) solves
this by running the actual `bun:sqlite` connection inside a Bun `Worker`
(`packages/data/src/sqlite.worker.ts`) and exposing a Promise-based client on the main thread —
every call is a `postMessage` round-trip, so a slow `SELECT` blocks the worker, never your app.

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

`filename` is caller-provided — an absolute path (composed with `ensureAppDataDir()` above),
`":memory:"`, or any path `bun:sqlite`'s `Database` constructor accepts; `options` (`readonly`,
`create`, `readwrite`) is passed straight through to it. `SqlParams` accepts either positional
bindings (`?`, `?1`, an array) or named bindings (`$id`/`:id`/`@id`, an object) — both cross the
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

### What this doesn't cover yet

The worker's protocol is deliberately raw SQL — `query`/`mutate`/`transaction`/`close` — not an ORM
hook. Nothing in `sqlite.worker.ts` wires up `drizzle-orm` or any other query builder today; if you
want one, build the SQL yourself (or generate it) and pass it through `query`/`mutate` as above.
