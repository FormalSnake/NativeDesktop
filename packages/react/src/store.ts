// Versioned JSON settings store: one `${name}.json` per store under the app
// data dir. The intended launch shape is `await store.load()` on the line
// above `render()`, which makes `get()` synchronous inside every component:
// no loading flash, no restore effect, no Suspense.
//
// Writes are debounced and serialized through one promise chain, land via
// write-to-tmp + rename so a crash mid-write can't corrupt the file, and a
// last-resort synchronous flush runs on exit/SIGINT/SIGTERM (the host kills
// the Bun child with SIGTERM, src/runtime.zig).

import { mkdirSync, renameSync, writeFileSync } from "node:fs";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { useMemo, useSyncExternalStore } from "./dev-react.ts";
import { getAppDataDir } from "./paths.ts";

export interface StoreOptions<T> {
  /** File is `${name}.json` under `dir` (default: getAppDataDir()). */
  name: string;
  version: number;
  defaults: T;
  /** Validate AND upgrade in one hook. Called on EVERY load, including a
   * current-version file (type guards and sanitizing live here, not in a
   * second option). Return null to reject the file and start from defaults. */
  migrate?: (raw: unknown, fromVersion: number) => T | null;
  /** Override for tests / env vars. */
  dir?: string;
  /** Default 250. */
  debounceMs?: number;
}

export interface Store<T> {
  readonly name: string;
  readonly path: string;
  /** Idempotent; repeat calls return the same promise. */
  load(): Promise<T>;
  /** Synchronous after load() resolves; throws a named error before that. */
  get(): T;
  set(next: T): void;
  update(fn: (prev: T) => T): void;
  subscribe(cb: (value: T) => void): () => void;
  /** Awaits the pending debounced write plus every queued one. */
  flush(): Promise<void>;
  /** Set when a corrupt/unmigratable file was backed up and reset. */
  readonly loadError: Error | undefined;
}

/** On-disk envelope; the version rides next to the data so `migrate` can
 * upgrade an old file without guessing what wrote it. */
interface Envelope {
  version: number;
  data: unknown;
}

class StoreImpl<T> implements Store<T> {
  readonly name: string;
  readonly path: string;
  readonly corruptPath: string;
  loadError: Error | undefined;

  #options: StoreOptions<T>;
  #value: T | undefined;
  #loaded = false;
  #loadPromise: Promise<T> | undefined;
  #subscribers = new Set<(value: T) => void>();
  #timer: ReturnType<typeof setTimeout> | undefined;
  #dirty = false;
  // Set while a queued async write is not yet durable (rename not resolved);
  // the exit flush must fire on this too, not just on #dirty — between the
  // debounce timer firing and the rename landing, the value exists only in
  // the write chain, and process.exit() would drop it.
  #pendingSnapshot: string | undefined;
  // Serializes writes so an older snapshot can never land after a newer one.
  #writeChain: Promise<void> = Promise.resolve();

  constructor(options: StoreOptions<T>, path: string) {
    this.#options = options;
    this.name = options.name;
    this.path = path;
    this.corruptPath = join(dirname(path), `${options.name}.corrupt.json`);
  }

  load(): Promise<T> {
    if (!this.#loadPromise) this.#loadPromise = this.#doLoad();
    return this.#loadPromise;
  }

  async #doLoad(): Promise<T> {
    let raw: string | undefined;
    try {
      raw = await readFile(this.path, "utf8");
    } catch {
      // Missing file is the first-launch case, not an error.
    }
    let value: T | undefined;
    if (raw !== undefined) {
      let parsed: unknown;
      let rejected: Error | undefined;
      try {
        parsed = JSON.parse(raw);
      } catch (err) {
        rejected = new Error(
          `nd: store "${this.name}" file is not valid JSON: ${err instanceof Error ? err.message : String(err)}`,
        );
      }
      if (!rejected) {
        const { data, fromVersion } = unwrapEnvelope(parsed);
        const migrate = this.#options.migrate;
        if (migrate) {
          let migrated: T | null;
          try {
            migrated = migrate(data, fromVersion);
          } catch (err) {
            migrated = null;
            rejected = new Error(
              `nd: store "${this.name}" migrate threw: ${err instanceof Error ? err.message : String(err)}`,
            );
          }
          if (migrated === null) {
            rejected ??= new Error(`nd: store "${this.name}" migrate rejected the file (version ${fromVersion})`);
          } else {
            value = migrated;
          }
        } else if (fromVersion === this.#options.version) {
          value = data as T;
        } else {
          rejected = new Error(
            `nd: store "${this.name}" is version ${fromVersion}, expected ${this.#options.version}, and no migrate hook is set`,
          );
        }
      }
      if (value === undefined) {
        // Single backup slot, overwritten: the file the user can still rescue
        // by hand, out of the way so the reset value can be written.
        this.loadError = rejected;
        try {
          await rename(this.path, this.corruptPath);
        } catch {
          // Backup is best-effort; the reset itself must not fail on it.
        }
      }
    }
    this.#value = value ?? this.#options.defaults;
    this.#loaded = true;
    return this.#value;
  }

  get(): T {
    if (!this.#loaded) throw this.#beforeLoadError();
    return this.#value as T;
  }

  set(next: T): void {
    if (!this.#loaded) throw this.#beforeLoadError();
    if (Object.is(next, this.#value)) return;
    this.#value = next;
    for (const cb of this.#subscribers) cb(next);
    this.#scheduleWrite();
  }

  update(fn: (prev: T) => T): void {
    this.set(fn(this.get()));
  }

  subscribe(cb: (value: T) => void): () => void {
    this.#subscribers.add(cb);
    return () => this.#subscribers.delete(cb);
  }

  flush(): Promise<void> {
    if (this.#timer) {
      clearTimeout(this.#timer);
      this.#timer = undefined;
    }
    if (this.#dirty) this.#queueWrite();
    return this.#writeChain;
  }

  #beforeLoadError(): Error {
    return new Error(`nd: store "${this.name}" read before load(); await store.load() before render()`);
  }

  #scheduleWrite(): void {
    this.#dirty = true;
    if (this.#timer) clearTimeout(this.#timer);
    this.#timer = setTimeout(() => {
      this.#timer = undefined;
      this.#queueWrite();
    }, this.#options.debounceMs ?? 250);
  }

  #queueWrite(): void {
    this.#dirty = false;
    const snapshot = this.#serialize();
    this.#pendingSnapshot = snapshot;
    this.#writeChain = this.#writeChain
      .then(async () => {
        await mkdir(dirname(this.path), { recursive: true });
        const tmp = `${this.path}.tmp-${process.pid}`;
        await writeFile(tmp, snapshot);
        await rename(tmp, this.path);
        // Only this link's own snapshot is durable now; a newer queued one
        // (identity check) must keep the exit flush armed.
        if (this.#pendingSnapshot === snapshot) this.#pendingSnapshot = undefined;
      })
      .catch((err) => {
        console.error(`[nd] store "${this.name}" write failed:`, err);
      });
  }

  #serialize(): string {
    const envelope: Envelope = { version: this.#options.version, data: this.#value };
    return JSON.stringify(envelope);
  }

  /** Exit-path flush: synchronous, no allocation of new timers, safe inside
   * a signal handler or the "exit" event. */
  writeSyncPending(): void {
    if (!this.#loaded || (!this.#dirty && this.#pendingSnapshot === undefined)) return;
    this.#dirty = false;
    this.#pendingSnapshot = undefined;
    if (this.#timer) {
      clearTimeout(this.#timer);
      this.#timer = undefined;
    }
    try {
      mkdirSync(dirname(this.path), { recursive: true });
      // Distinct tmp name: an already-submitted async writeFile may still be
      // mid-flight on the threadpool, and interleaving two writers on one
      // tmp file could rename garbage over the real file.
      const tmp = `${this.path}.tmp-exit-${process.pid}`;
      writeFileSync(tmp, this.#serialize());
      renameSync(tmp, this.path);
    } catch (err) {
      console.error(`[nd] store "${this.name}" exit flush failed:`, err);
    }
  }
}

function unwrapEnvelope(parsed: unknown): { data: unknown; fromVersion: number } {
  if (
    typeof parsed === "object" &&
    parsed !== null &&
    typeof (parsed as Envelope).version === "number" &&
    "data" in (parsed as Envelope)
  ) {
    return { data: (parsed as Envelope).data, fromVersion: (parsed as Envelope).version };
  }
  // Pre-envelope or hand-written file: hand the whole value to migrate as v0.
  return { data: parsed, fromVersion: 0 };
}

// globalThis-keyed registry so `nd dev` hot re-eval reuses the loaded
// instance (and its pending debounce timer) instead of re-reading a file the
// in-memory value is already ahead of. Also keeps the exit hooks singular.
interface StoreRegistry {
  stores: Map<string, StoreImpl<unknown>>;
  exitHooked: boolean;
}

declare global {
  // eslint-disable-next-line no-var
  var __nd_stores: StoreRegistry | undefined;
}

function registry(): StoreRegistry {
  if (!globalThis.__nd_stores) globalThis.__nd_stores = { stores: new Map(), exitHooked: false };
  return globalThis.__nd_stores;
}

function hookExit(reg: StoreRegistry): void {
  if (reg.exitHooked) return;
  reg.exitHooked = true;
  const flushAll = (): void => {
    for (const store of reg.stores.values()) store.writeSyncPending();
  };
  process.once("exit", flushAll);
  for (const signal of ["SIGINT", "SIGTERM"] as const) {
    process.once(signal, () => {
      flushAll();
      // The once-handler is already removed here; if the app installed its
      // own handler it now owns shutdown, otherwise exit with the
      // conventional code so the signal still terminates the process.
      if (process.listenerCount(signal) === 0) process.exit(signal === "SIGINT" ? 130 : 143);
    });
  }
}

/** Deduped by resolved file path: the same name+dir always returns the same
 * instance (a `dir` override in tests is a distinct instance). */
export function createStore<T>(options: StoreOptions<T>): Store<T> {
  const dir = options.dir ?? getAppDataDir();
  const path = resolve(join(dir, `${options.name}.json`));
  const reg = registry();
  const existing = reg.stores.get(path);
  if (existing) return existing as unknown as Store<T>;
  const store = new StoreImpl<T>(options, path);
  reg.stores.set(path, store as unknown as StoreImpl<unknown>);
  hookExit(reg);
  return store;
}

/** Subscribes a component to the store (or a selection of it). The selected
 * snapshot is cached per store value, so `select` may return a fresh object
 * without re-render loops, but it must be pure. */
export function useStoreValue<T, S = T>(store: Store<T>, select?: (value: T) => S): S {
  const getSnapshot = useMemo(() => {
    let lastValue: T | undefined;
    let lastSelected: S;
    let primed = false;
    return (): S => {
      const value = store.get();
      if (!primed || !Object.is(value, lastValue)) {
        lastValue = value;
        lastSelected = select ? select(value) : (value as unknown as S);
        primed = true;
      }
      return lastSelected;
    };
  }, [store, select]);
  const subscribe = useMemo(() => {
    return (onChange: () => void) => store.subscribe(() => onChange());
  }, [store]);
  return useSyncExternalStore(subscribe, getSnapshot);
}
