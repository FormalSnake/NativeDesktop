import { beforeEach, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// homedir() ignores a HOME set after startup, so every lookup runs in a child
// with its own HOME; an in-process call would resolve the real profile.
let root: string;

function dataRoot(): string {
  return process.platform === "darwin" ? join(root, "home", "Library", "Application Support") : join(root, "data");
}

function lookup(env: Record<string, string> = {}, fn = "getAppDataDir"): string {
  const proc = Bun.spawnSync(
    ["bun", "-e", `import { ${fn} } from ${JSON.stringify(join(import.meta.dir, "paths.ts"))}; console.log(${fn}())`],
    {
      cwd: join(root, "app"),
      env: { PATH: process.env.PATH ?? "", HOME: join(root, "home"), XDG_DATA_HOME: join(root, "data"), ...env },
    },
  );
  if (proc.exitCode !== 0) throw new Error(proc.stderr.toString());
  return proc.stdout.toString().trim();
}

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), "nd-paths-"));
  mkdirSync(join(root, "app"));
  writeFileSync(join(root, "app", "package.json"), JSON.stringify({ name: "lynk" }));
  mkdirSync(dataRoot(), { recursive: true });
});

describe("getAppDataDir", () => {
  test("dev and packaged runs resolve the same directory", () => {
    const dev = lookup();
    writeFileSync(join(root, "app", "nd-app.json"), JSON.stringify({ name: "Lynk Browser", dataName: "lynk" }));
    expect(lookup()).toBe(dev);
    expect(dev).toBe(join(dataRoot(), "lynk"));
  });

  test("each app gets its own chromium profile, shared by its dev and packaged runs", () => {
    const dev = lookup({}, "getCefProfileDir");
    expect(dev).toBe(join(dataRoot(), "lynk", "cef"));
    writeFileSync(join(root, "app", "nd-app.json"), JSON.stringify({ name: "Lynk Browser", dataName: "lynk" }));
    expect(lookup({}, "getCefProfileDir")).toBe(dev);
    writeFileSync(join(root, "app", "nd-app.json"), JSON.stringify({ name: "Other", dataName: "other" }));
    expect(lookup({}, "getCefProfileDir")).toBe(join(dataRoot(), "other", "cef"));
  });

  test("ND_CEF_CACHE overrides the chromium profile", () => {
    expect(lookup({ ND_CEF_CACHE: "/tmp/rig/cef" }, "getCefProfileDir")).toBe("/tmp/rig/cef");
  });
});
