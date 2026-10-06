// Per-app user-data directory, following each OS's native convention: the
// equivalent of Electron's `app.getPath('userData')`. In a packaged app the
// name comes from the bundle's nd-app.json (written by `nd package`, found by
// walking up from cwd); in dev it comes from the nearest `package.json`'s
// `name` field, so apps that configure no app.name keep their existing dirs.

import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, resolve } from "node:path";

function appName(): string {
  let dir = process.cwd();
  while (true) {
    const manifest = resolve(dir, "nd-app.json");
    if (existsSync(manifest)) {
      const name = (JSON.parse(readFileSync(manifest, "utf8")) as { name?: string }).name;
      if (name) return name;
      break;
    }
    const parent = dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  const pkg = JSON.parse(readFileSync(resolve(process.cwd(), "package.json"), "utf8")) as { name?: string };
  if (!pkg.name) throw new Error("nd: could not resolve app name from package.json");
  return pkg.name;
}

/** The per-platform user-data directory for this app. Does not create it. */
export function getAppDataDir(): string {
  const name = appName();
  switch (process.platform) {
    case "darwin":
      return resolve(homedir(), "Library", "Application Support", name);
    case "win32":
      return resolve(process.env.APPDATA ?? homedir(), name);
    default:
      return resolve(process.env.XDG_DATA_HOME || resolve(homedir(), ".local", "share"), name);
  }
}

/** Like `getAppDataDir`, but also creates the directory (recursively) if missing. */
export function ensureAppDataDir(): string {
  const dir = getAppDataDir();
  mkdirSync(dir, { recursive: true });
  return dir;
}
