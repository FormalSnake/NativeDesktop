import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";

export const jsxFile = /\.[jt]sx$/;

// A file outside any Solid package opts in per file with the pragma
// TypeScript reads as well: /** @jsxImportSource @nativedesktop/solid */.
const pragma = /@jsxImportSource\s+@nativedesktop\/solid\b/;

/** True when `source` at `path` is Solid JSX, by its package or its pragma. */
export function isSolidModule(path: string, source: string): boolean {
  return isSolidSource(path) || pragma.test(source);
}

// A Solid .tsx is one whose nearest package.json is this package or depends
// on it; any other .tsx (React code in the same process, as under a
// repo-wide `bun test`) keeps Bun's own transform.
const solidPackages = new Map<string, boolean>();
export function isSolidSource(path: string): boolean {
  for (let dir = dirname(path); ; dir = dirname(dir)) {
    const known = solidPackages.get(dir);
    if (known !== undefined) return known;
    const manifest = join(dir, "package.json");
    if (existsSync(manifest)) {
      const pkg = JSON.parse(readFileSync(manifest, "utf8")) as {
        name?: string;
        dependencies?: Record<string, string>;
        devDependencies?: Record<string, string>;
      };
      const solid =
        pkg.name === "@nativedesktop/solid" ||
        pkg.dependencies?.["@nativedesktop/solid"] !== undefined ||
        pkg.devDependencies?.["@nativedesktop/solid"] !== undefined;
      solidPackages.set(dir, solid);
      return solid;
    }
    if (dirname(dir) === dir) return false;
  }
}
