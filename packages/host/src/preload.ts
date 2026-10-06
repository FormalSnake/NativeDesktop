// The Bun preload a renderer needs before the app's first module evaluates.
// A Solid app's JSX goes through babel-preset-solid and its solid-js import
// has to land on the client build, both done by @nativedesktop/solid/register,
// a runtime Bun plugin that only takes effect for modules loaded after it.
// The host spawns `bun [--hot] <entry>` itself, so the preload reaches that
// child through BUN_OPTIONS, which Bun reads for every invocation.
//
// The preload is passed as a bare specifier, which Bun resolves from the
// child's cwd. BUN_OPTIONS has no reliable quoting (a quoted token is dropped
// unless it comes first), so an absolute path would break on a bundle or
// checkout path with a space in it.
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

export const SOLID_PRELOAD = "@nativedesktop/solid/register";

/** True when the app's package.json lists @nativedesktop/solid as a runtime dependency. */
export function usesSolid(appDir: string): boolean {
  const pkgPath = join(appDir, "package.json");
  if (!existsSync(pkgPath)) return false;
  const pkg = JSON.parse(readFileSync(pkgPath, "utf8")) as { dependencies?: Record<string, string> };
  return pkg.dependencies?.["@nativedesktop/solid"] !== undefined;
}

/** The module specifier the app's renderer needs preloaded, or undefined. */
export function rendererPreload(appDir: string): string | undefined {
  return usesSolid(appDir) ? SOLID_PRELOAD : undefined;
}

/** `existing` BUN_OPTIONS with a `--preload` for `specifier` appended. */
export function withPreload(existing: string | undefined, specifier: string): string {
  const flag = `--preload=${specifier}`;
  if (!existing?.trim()) return flag;
  return existing.split(/\s+/).includes(flag) ? existing : `${existing} ${flag}`;
}

/** The env entries that give the host's bun child the app's renderer preload. */
export function preloadEnv(appDir: string, existing: string | undefined = process.env.BUN_OPTIONS): Record<string, string> {
  const specifier = rendererPreload(appDir);
  return specifier ? { BUN_OPTIONS: withPreload(existing, specifier) } : {};
}

/** The nearest directory at or above `from` that holds a package.json. */
export function findAppDir(from: string): string | undefined {
  for (let dir = resolve(from); ; dir = dirname(dir)) {
    if (existsSync(join(dir, "package.json"))) return dir;
    if (dirname(dir) === dir) return undefined;
  }
}
