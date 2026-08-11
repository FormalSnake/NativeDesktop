// Payload assembly units: entry rewriting, workspace mapping, `nd package`
// arg parsing, and an end-to-end assemblePayload run against a temp fixture.
import { describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parsePackageArgs } from "../index.ts";
import { assemblePayload, compiledEntryFor, workspaceRelative } from "./payload.ts";
import type { ResolvedIdentity } from "./identity.ts";

describe("compiledEntryFor", () => {
  test("replaces the first path segment with the compile outDir", () => {
    expect(compiledEntryFor("src/main.tsx", "dist")).toBe("dist/main.tsx");
    expect(compiledEntryFor("src/app/main.tsx", "out")).toBe("out/app/main.tsx");
  });

  test("rejects a root-level entry (nothing to substitute)", () => {
    expect(() => compiledEntryFor("main.tsx", "dist")).toThrow("compiled entry");
  });
});

describe("workspaceRelative", () => {
  test("maps the app dir to its workspace-relative path", () => {
    expect(workspaceRelative("/repo", "/repo/examples/gallery")).toBe("examples/gallery");
    expect(workspaceRelative("/repo/app", "/repo/app")).toBe("");
  });

  test("rejects an app dir outside the workspace root", () => {
    expect(() => workspaceRelative("/repo/app", "/elsewhere")).toThrow("outside workspaceRoot");
  });
});

describe("parsePackageArgs", () => {
  test("platform defaults to the host and flags map onto PackageOptions", () => {
    expect(parsePackageArgs([], "darwin").platform).toBe("mac");
    expect(parsePackageArgs([], "linux").platform).toBe("linux");
    const opts = parsePackageArgs(
      ["linux", "--out", "build", "--entry", "app.tsx", "--version", "2.0.0", "--cwd", "apps/x", "--no-compile", "--format", "appdir"],
      "darwin",
    );
    expect(opts).toEqual({
      platform: "linux",
      outDir: "build",
      entry: "app.tsx",
      version: "2.0.0",
      cwd: "apps/x",
      compile: false,
      format: "appdir",
    });
  });

  test("--sign/--no-sign and --notarize/--no-notarize resolve to identity overrides", () => {
    expect(parsePackageArgs(["mac", "--sign", "Developer ID"], "darwin").signIdentity).toBe("Developer ID");
    expect(parsePackageArgs(["mac", "--no-sign"], "darwin").signIdentity).toBeNull();
    expect(parsePackageArgs(["mac", "--notarize"], "darwin").notarize).toBe(true);
    expect(parsePackageArgs(["mac", "--no-notarize"], "darwin").notarize).toBe(false);
  });
});

describe("assemblePayload", () => {
  const identity: ResolvedIdentity = {
    id: "com.example.fixture",
    name: "Fixture",
    displayName: "Fixture",
    slug: "fixture",
    version: "1.0.0",
    categories: ["Utility"],
  };

  test("copies the entry dir, writes nd-app.json, and mirrors the workspace layout", async () => {
    const root = mkdtempSync(join(tmpdir(), "nd-payload-"));
    const appDir = join(root, "apps", "fixture");
    mkdirSync(join(appDir, "src"), { recursive: true });
    writeFileSync(join(appDir, "package.json"), JSON.stringify({ name: "fixture", version: "1.0.0" }));
    writeFileSync(join(appDir, "src", "main.tsx"), "export {};\n");
    mkdirSync(join(root, "shared"), { recursive: true });
    writeFileSync(join(root, "shared", "util.ts"), "export {};\n");

    const appRoot = join(root, "out", "app");
    const result = await assemblePayload({
      appDir,
      config: { package: { workspaceRoot: "../..", include: ["shared"], compile: false } },
      identity,
      appRoot,
    });

    expect(result).toEqual({ entry: "apps/fixture/src/main.tsx", cwd: "apps/fixture", pluginPaths: [] });
    expect(existsSync(join(appRoot, "apps/fixture/src/main.tsx"))).toBe(true);
    expect(existsSync(join(appRoot, "apps/fixture/package.json"))).toBe(true);
    expect(existsSync(join(appRoot, "shared/util.ts"))).toBe(true);
    const manifest = JSON.parse(readFileSync(join(appRoot, "nd-app.json"), "utf8"));
    expect(manifest).toEqual({
      id: "com.example.fixture",
      name: "Fixture",
      version: "1.0.0",
      entry: "apps/fixture/src/main.tsx",
      cwd: "apps/fixture",
      pluginPaths: [],
    });
  });

  test("default config: bundle output nested in the compile outDir is not copied into itself", async () => {
    // The template shape: no `package` block, a `compile` script emitting
    // dist/, so the bundle output (<appDir>/dist/mac/...) nests inside the
    // copied entry dir (<appDir>/dist).
    const appDir = mkdtempSync(join(tmpdir(), "nd-payload-default-"));
    mkdirSync(join(appDir, "src"), { recursive: true });
    writeFileSync(join(appDir, "package.json"), JSON.stringify({
      name: "fixture",
      version: "1.0.0",
      scripts: { compile: "mkdir -p dist && cp src/main.tsx dist/main.tsx" },
    }));
    writeFileSync(join(appDir, "src", "main.tsx"), "export {};\n");

    // appRoot exactly as mac.ts derives it from the default outDir.
    const contents = join(appDir, "dist", "mac", "Fixture.app", "Contents");
    for (const dir of ["MacOS", "Resources", "Frameworks"]) mkdirSync(join(contents, dir), { recursive: true });
    const appRoot = join(contents, "Resources", "app");

    const result = await assemblePayload({ appDir, config: {}, identity, appRoot });

    expect(result).toEqual({ entry: "dist/main.tsx", cwd: ".", pluginPaths: [] });
    expect(existsSync(join(appRoot, "dist/main.tsx"))).toBe(true);
    expect(existsSync(join(appRoot, "package.json"))).toBe(true);
    expect(existsSync(join(appRoot, "dist", "mac"))).toBe(false);
  });

  test("root-level entry ships the whole app dir minus node_modules", async () => {
    const root = mkdtempSync(join(tmpdir(), "nd-payload-"));
    const appDir = join(root, "app");
    mkdirSync(join(appDir, "assets"), { recursive: true });
    mkdirSync(join(appDir, "node_modules", "junk"), { recursive: true });
    writeFileSync(join(appDir, "package.json"), JSON.stringify({ name: "flat", version: "1.0.0" }));
    writeFileSync(join(appDir, "main.tsx"), "export {};\n");
    writeFileSync(join(appDir, "assets", "a.txt"), "a\n");

    const appRoot = join(root, "out", "app");
    const result = await assemblePayload({
      appDir,
      config: { package: { entry: "main.tsx", compile: false } },
      identity,
      appRoot,
    });

    expect(result).toEqual({ entry: "main.tsx", cwd: ".", pluginPaths: [] });
    expect(existsSync(join(appRoot, "main.tsx"))).toBe(true);
    expect(existsSync(join(appRoot, "assets/a.txt"))).toBe(true);
    expect(existsSync(join(appRoot, "node_modules", "junk"))).toBe(false);
  });
});
