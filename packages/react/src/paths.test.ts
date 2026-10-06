import { beforeEach, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// homedir() ignores a HOME set after startup, so every lookup runs in a child
// with its own HOME; an in-process call would resolve the real profile.
let root: string;

function dataRoot(): string {
  return process.platform === "darwin" ? join(root, "home", "Library", "Application Support") : join(root, "data");
}

function lookup(env: Record<string, string> = {}): string {
  const proc = Bun.spawnSync(
    ["bun", "-e", `import { getAppDataDir } from ${JSON.stringify(join(import.meta.dir, "paths.ts"))}; console.log(getAppDataDir())`],
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

  test("a renamed app moves the old directory across once", () => {
    mkdirSync(join(dataRoot(), "nativebrowser"));
    writeFileSync(join(dataRoot(), "nativebrowser", "session.json"), "{}");
    const dir = lookup({ ND_APP_PREVIOUS_NAME: "NativeBrowser" });
    expect(readFileSync(join(dir, "session.json"), "utf8")).toBe("{}");
    mkdirSync(join(dataRoot(), "nativebrowser"));
    lookup({ ND_APP_PREVIOUS_NAME: "NativeBrowser" });
    expect(existsSync(join(dataRoot(), "nativebrowser"))).toBe(true);
  });

  test("a packaged run takes the previous name from its manifest", () => {
    writeFileSync(join(root, "app", "nd-app.json"), JSON.stringify({ name: "Lynk Browser", dataName: "lynk", previousName: "NativeBrowser" }));
    mkdirSync(join(dataRoot(), "NativeBrowser"));
    writeFileSync(join(dataRoot(), "NativeBrowser", "history.sqlite"), "");
    expect(lookup()).toBe(join(dataRoot(), "lynk"));
    expect(existsSync(join(dataRoot(), "lynk", "history.sqlite"))).toBe(true);
  });
});
