import { describe, expect, test } from "bun:test";
import { existsSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createStore } from "./store.ts";

interface Settings {
  theme: string;
  count: number;
}

const DEFAULTS: Settings = { theme: "light", count: 0 };

let nameSeq = 0;

function freshDir(): string {
  return mkdtempSync(join(tmpdir(), "nd-store-test-"));
}

function freshName(): string {
  return `s${++nameSeq}`;
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

describe("createStore", () => {
  test("missing file loads defaults with no loadError", async () => {
    const store = createStore<Settings>({ name: freshName(), version: 1, defaults: DEFAULTS, dir: freshDir() });
    expect(await store.load()).toEqual(DEFAULTS);
    expect(store.loadError).toBeUndefined();
  });

  test("get() before load() throws the named error", () => {
    const name = freshName();
    const store = createStore<Settings>({ name, version: 1, defaults: DEFAULTS, dir: freshDir() });
    expect(() => store.get()).toThrow(`nd: store "${name}" read before load(); await store.load() before render()`);
  });

  test("load() is idempotent: repeat calls return the same promise", () => {
    const store = createStore<Settings>({ name: freshName(), version: 1, defaults: DEFAULTS, dir: freshDir() });
    expect(store.load()).toBe(store.load());
  });

  test("migrate is called on a current-version file too, with the envelope's version", async () => {
    const dir = freshDir();
    const name = freshName();
    writeFileSync(join(dir, `${name}.json`), JSON.stringify({ version: 1, data: { theme: "dark", count: 3 } }));
    const seen: number[] = [];
    const store = createStore<Settings>({
      name,
      version: 1,
      defaults: DEFAULTS,
      dir,
      migrate: (raw, fromVersion) => {
        seen.push(fromVersion);
        return raw as Settings;
      },
    });
    expect(await store.load()).toEqual({ theme: "dark", count: 3 });
    expect(seen).toEqual([1]);
  });

  test("migrate upgrades an old version", async () => {
    const dir = freshDir();
    const name = freshName();
    writeFileSync(join(dir, `${name}.json`), JSON.stringify({ version: 0, data: { theme: "dark" } }));
    const store = createStore<Settings>({
      name,
      version: 1,
      defaults: DEFAULTS,
      dir,
      migrate: (raw, fromVersion) => {
        const old = raw as { theme?: string };
        if (fromVersion === 0) return { theme: old.theme ?? "light", count: 0 };
        return raw as Settings;
      },
    });
    expect(await store.load()).toEqual({ theme: "dark", count: 0 });
    expect(store.loadError).toBeUndefined();
  });

  test("corrupt JSON is backed up to the single .corrupt.json slot and reset to defaults", async () => {
    const dir = freshDir();
    const name = freshName();
    writeFileSync(join(dir, `${name}.json`), "{not json");
    const store = createStore<Settings>({ name, version: 1, defaults: DEFAULTS, dir });
    expect(await store.load()).toEqual(DEFAULTS);
    expect(store.loadError).toBeInstanceOf(Error);
    expect(readFileSync(join(dir, `${name}.corrupt.json`), "utf8")).toBe("{not json");
    expect(existsSync(join(dir, `${name}.json`))).toBe(false);
  });

  test("migrate returning null backs up and resets", async () => {
    const dir = freshDir();
    const name = freshName();
    const original = JSON.stringify({ version: 1, data: "garbage" });
    writeFileSync(join(dir, `${name}.json`), original);
    const store = createStore<Settings>({
      name,
      version: 1,
      defaults: DEFAULTS,
      dir,
      migrate: () => null,
    });
    expect(await store.load()).toEqual(DEFAULTS);
    expect(store.loadError).toBeInstanceOf(Error);
    expect(readFileSync(join(dir, `${name}.corrupt.json`), "utf8")).toBe(original);
  });

  test("version mismatch without a migrate hook backs up and resets", async () => {
    const dir = freshDir();
    const name = freshName();
    writeFileSync(join(dir, `${name}.json`), JSON.stringify({ version: 7, data: { theme: "dark", count: 1 } }));
    const store = createStore<Settings>({ name, version: 1, defaults: DEFAULTS, dir });
    expect(await store.load()).toEqual(DEFAULTS);
    expect(store.loadError?.message).toContain("version 7");
    expect(existsSync(join(dir, `${name}.corrupt.json`))).toBe(true);
  });

  test("set() debounces: rapid sets produce one file with the last value", async () => {
    const dir = freshDir();
    const name = freshName();
    const store = createStore<Settings>({ name, version: 1, defaults: DEFAULTS, dir, debounceMs: 20 });
    await store.load();
    store.set({ theme: "a", count: 1 });
    store.set({ theme: "b", count: 2 });
    expect(existsSync(join(dir, `${name}.json`))).toBe(false);
    await sleep(60);
    const envelope = JSON.parse(readFileSync(join(dir, `${name}.json`), "utf8"));
    expect(envelope).toEqual({ version: 1, data: { theme: "b", count: 2 } });
  });

  test("writes are atomic: no .tmp- file survives a flush", async () => {
    const dir = freshDir();
    const name = freshName();
    const store = createStore<Settings>({ name, version: 1, defaults: DEFAULTS, dir, debounceMs: 5 });
    await store.load();
    store.set({ theme: "x", count: 9 });
    await store.flush();
    const leftovers = readdirSync(dir).filter((f) => f.includes(".tmp-"));
    expect(leftovers).toEqual([]);
    expect(JSON.parse(readFileSync(join(dir, `${name}.json`), "utf8")).data.count).toBe(9);
  });

  test("flush ordering: an older snapshot never lands after a newer one", async () => {
    const dir = freshDir();
    const name = freshName();
    const store = createStore<Settings>({ name, version: 1, defaults: DEFAULTS, dir, debounceMs: 5 });
    await store.load();
    store.set({ theme: "first", count: 1 });
    const first = store.flush();
    store.set({ theme: "second", count: 2 });
    const second = store.flush();
    await Promise.all([first, second]);
    expect(JSON.parse(readFileSync(join(dir, `${name}.json`), "utf8")).data.theme).toBe("second");
  });

  test("flush() with nothing pending resolves without writing", async () => {
    const dir = freshDir();
    const name = freshName();
    const store = createStore<Settings>({ name, version: 1, defaults: DEFAULTS, dir });
    await store.load();
    await store.flush();
    expect(existsSync(join(dir, `${name}.json`))).toBe(false);
  });

  test("dedupe key is the resolved path: same name+dir shares, different dir is distinct", async () => {
    const name = freshName();
    const dir = freshDir();
    const a = createStore<Settings>({ name, version: 1, defaults: DEFAULTS, dir });
    const b = createStore<Settings>({ name, version: 1, defaults: DEFAULTS, dir });
    const c = createStore<Settings>({ name, version: 1, defaults: DEFAULTS, dir: freshDir() });
    expect(a).toBe(b);
    expect(c).not.toBe(a);
  });

  test("subscribe() notifies on set and stops after unsubscribe", async () => {
    const store = createStore<Settings>({ name: freshName(), version: 1, defaults: DEFAULTS, dir: freshDir() });
    await store.load();
    const seen: Settings[] = [];
    const unsubscribe = store.subscribe((v) => seen.push(v));
    store.set({ theme: "a", count: 1 });
    unsubscribe();
    store.set({ theme: "b", count: 2 });
    expect(seen).toEqual([{ theme: "a", count: 1 }]);
    expect(store.get()).toEqual({ theme: "b", count: 2 });
    await store.flush();
  });

  test("update() applies against the current value", async () => {
    const store = createStore<Settings>({ name: freshName(), version: 1, defaults: DEFAULTS, dir: freshDir() });
    await store.load();
    store.update((prev) => ({ ...prev, count: prev.count + 1 }));
    store.update((prev) => ({ ...prev, count: prev.count + 1 }));
    expect(store.get().count).toBe(2);
    await store.flush();
  });
});
