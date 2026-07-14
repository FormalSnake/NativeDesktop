// Per-app user-data directory, following each OS's native convention — the
// equivalent of Electron's `app.getPath('userData')`. NativeDesktop has no
// separate app-identity config yet (`NativeDesktopConfig` in nd/config.ts
// carries only `native.plugins`), so the app name comes from the nearest
// `package.json`'s `name` field, read from the same cwd `loadConfig()`
// resolves `nativedesktop.config.ts` from.

import { mkdirSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { resolve } from "node:path";

function appName(): string {
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
