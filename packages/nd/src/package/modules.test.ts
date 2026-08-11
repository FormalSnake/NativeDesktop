// flattenRuntimeModules against a Bun-isolated-store-shaped fixture: packages
// live under node_modules/.bun/<pkg>@<ver>/node_modules/<pkg> with sibling
// symlinks for their deps, and the app's node_modules only symlinks its direct
// deps. The flattener must produce one flat, real-file node_modules.
import { describe, expect, test } from "bun:test";
import { existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { assertResolvableEntries, flattenRuntimeModules } from "./modules.ts";

interface FixturePkg {
  name: string;
  version: string;
  dependencies?: Record<string, string>;
  peerDependencies?: Record<string, string>;
  main?: string;
}

/** Creates <root>/node_modules/.bun/<name>@<version>/node_modules/<name>. */
function storePackage(root: string, pkg: FixturePkg): string {
  const dir = join(root, "node_modules", ".bun", `${pkg.name}@${pkg.version}`, "node_modules", pkg.name);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "package.json"), JSON.stringify({ main: "index.js", ...pkg }));
  writeFileSync(join(dir, pkg.main ?? "index.js"), `module.exports = "${pkg.name}@${pkg.version}";\n`);
  return dir;
}

/** Symlinks depName next to the store entry of `fromPkg` (Bun's sibling layout). */
function storeSibling(root: string, fromPkg: string, depName: string, depDir: string): void {
  const link = join(root, "node_modules", ".bun", fromPkg, "node_modules", depName);
  mkdirSync(join(link, ".."), { recursive: true });
  symlinkSync(depDir, link);
}

function makeFixture(): { appDir: string; root: string } {
  const root = mkdtempSync(join(tmpdir(), "nd-modules-"));
  const appDir = join(root, "app");
  mkdirSync(appDir, { recursive: true });
  writeFileSync(join(appDir, "package.json"), JSON.stringify({
    name: "fixture-app",
    dependencies: { "dep-a": "1.0.0", "dep-c": "1.0.0" },
  }));

  const depA = storePackage(root, { name: "dep-a", version: "1.0.0", dependencies: { "dep-b": "1.0.0" } });
  const depB1 = storePackage(root, { name: "dep-b", version: "1.0.0" });
  const depB2 = storePackage(root, { name: "dep-b", version: "2.0.0" });
  const depC = storePackage(root, { name: "dep-c", version: "1.0.0", dependencies: { "dep-b": "2.0.0" } });
  storeSibling(root, "dep-a@1.0.0", "dep-b", depB1);
  storeSibling(root, "dep-c@1.0.0", "dep-b", depB2);

  mkdirSync(join(appDir, "node_modules"), { recursive: true });
  symlinkSync(depA, join(appDir, "node_modules", "dep-a"));
  symlinkSync(depC, join(appDir, "node_modules", "dep-c"));
  return { appDir, root };
}

describe("flattenRuntimeModules", () => {
  test("flattens the isolated store into real directories and nests version conflicts", () => {
    const { appDir, root } = makeFixture();
    const dest = join(root, "out", "node_modules");
    const flat = flattenRuntimeModules({ appDir, dest });

    expect(flat.sort()).toEqual(["dep-a", "dep-b", "dep-c"]);
    for (const name of ["dep-a", "dep-b", "dep-c"]) {
      const dir = join(dest, name);
      expect(existsSync(join(dir, "package.json"))).toBe(true);
      expect(lstatSync(dir).isSymbolicLink()).toBe(false);
    }
    // dep-a claimed the flat dep-b@1 slot first; dep-c's dep-b@2 nests.
    expect(JSON.parse(readFileSync(join(dest, "dep-b", "package.json"), "utf8")).version).toBe("1.0.0");
    const nested = join(dest, "dep-c", "node_modules", "dep-b", "package.json");
    expect(existsSync(nested)).toBe(true);
    expect(JSON.parse(readFileSync(nested, "utf8")).version).toBe("2.0.0");
  });

  test("terminates on a dependency cycle with version conflicts and reuses reachable copies", () => {
    // a@1 <-> b@1 at the top, with b@1 -> a@2 -> b@2 -> a@2 cycling on
    // conflicting versions. Nesting must stop once the needed version already
    // resolves from an ancestor slot.
    const root = mkdtempSync(join(tmpdir(), "nd-modules-"));
    const appDir = join(root, "app");
    mkdirSync(appDir, { recursive: true });
    writeFileSync(join(appDir, "package.json"), JSON.stringify({
      name: "fixture-app",
      dependencies: { "dep-a": "1.0.0", "dep-b": "1.0.0" },
    }));
    const a1 = storePackage(root, { name: "dep-a", version: "1.0.0", dependencies: { "dep-b": "1.0.0" } });
    const b1 = storePackage(root, { name: "dep-b", version: "1.0.0", dependencies: { "dep-a": "2.0.0" } });
    const a2 = storePackage(root, { name: "dep-a", version: "2.0.0", dependencies: { "dep-b": "2.0.0" } });
    const b2 = storePackage(root, { name: "dep-b", version: "2.0.0", dependencies: { "dep-a": "2.0.0" } });
    storeSibling(root, "dep-a@1.0.0", "dep-b", b1);
    storeSibling(root, "dep-b@1.0.0", "dep-a", a2);
    storeSibling(root, "dep-a@2.0.0", "dep-b", b2);
    storeSibling(root, "dep-b@2.0.0", "dep-a", a2);
    mkdirSync(join(appDir, "node_modules"), { recursive: true });
    symlinkSync(a1, join(appDir, "node_modules", "dep-a"));
    symlinkSync(b1, join(appDir, "node_modules", "dep-b"));

    const dest = join(root, "out", "node_modules");
    const flat = flattenRuntimeModules({ appDir, dest });
    expect(flat.sort()).toEqual(["dep-a", "dep-b"]);
    const version = (p: string) => JSON.parse(readFileSync(join(dest, p, "package.json"), "utf8")).version;
    expect(version("dep-a")).toBe("1.0.0");
    expect(version("dep-b")).toBe("1.0.0");
    expect(version("dep-b/node_modules/dep-a")).toBe("2.0.0");
    expect(version("dep-b/node_modules/dep-a/node_modules/dep-b")).toBe("2.0.0");
    // b@2's dep-a@2 resolves via the copy two levels up: no deeper nesting.
    expect(existsSync(join(dest, "dep-b/node_modules/dep-a/node_modules/dep-b/node_modules/dep-a"))).toBe(false);
  });

  test("skips unresolvable peer dependencies but throws on a missing hard dependency", () => {
    const root = mkdtempSync(join(tmpdir(), "nd-modules-"));
    const appDir = join(root, "app");
    mkdirSync(appDir, { recursive: true });
    writeFileSync(join(appDir, "package.json"), JSON.stringify({ name: "a", dependencies: { "dep-p": "1.0.0" } }));
    const depP = storePackage(root, { name: "dep-p", version: "1.0.0", peerDependencies: { "not-installed": "*" } });
    mkdirSync(join(appDir, "node_modules"), { recursive: true });
    symlinkSync(depP, join(appDir, "node_modules", "dep-p"));

    const dest = join(root, "out", "node_modules");
    expect(flattenRuntimeModules({ appDir, dest }).sort()).toEqual(["dep-p"]);

    writeFileSync(join(appDir, "package.json"), JSON.stringify({ name: "a", dependencies: { "dep-p": "1.0.0", missing: "*" } }));
    expect(() => flattenRuntimeModules({ appDir, dest: join(root, "out2", "node_modules") }))
      .toThrow('cannot resolve runtime dependency "missing"');
  });
});

describe("assertResolvableEntries", () => {
  test("throws a build hint when a copied package's entry target is missing", () => {
    const root = mkdtempSync(join(tmpdir(), "nd-modules-"));
    const broken = join(root, "node_modules", "needs-build");
    mkdirSync(broken, { recursive: true });
    writeFileSync(join(broken, "package.json"), JSON.stringify({ name: "needs-build", main: "./dist/index.js" }));
    expect(() => assertResolvableEntries(root, ["needs-build"])).toThrow("bun run build");
  });

  test("tolerates a package that declares no entry point (binary carrier)", () => {
    const root = mkdtempSync(join(tmpdir(), "nd-modules-"));
    const carrier = join(root, "node_modules", "host-binary");
    mkdirSync(carrier, { recursive: true });
    writeFileSync(join(carrier, "package.json"), JSON.stringify({ name: "host-binary", files: ["bin"] }));
    expect(() => assertResolvableEntries(root, ["host-binary"])).not.toThrow();
  });
});
