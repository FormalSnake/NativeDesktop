// Per-app user-data directory, following each OS's native convention: the
// equivalent of Electron's `app.getPath('userData')`. The directory is named
// after the app's package.json `name` in dev and packaged runs alike (a packaged
// app finds it in the bundle's nd-app.json, written by `nd package` and found by
// walking up from cwd), so both modes share one profile.

import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";

interface BundleManifest {
  name?: string;
  dataName?: string;
}

function bundleManifest(): BundleManifest | undefined {
  let dir = process.cwd();
  while (true) {
    const manifest = resolve(dir, "nd-app.json");
    if (existsSync(manifest)) return JSON.parse(readFileSync(manifest, "utf8")) as BundleManifest;
    const parent = dirname(dir);
    if (parent === dir) return undefined;
    dir = parent;
  }
}

function appName(): string {
  const manifest = bundleManifest();
  // Bundles packaged before dataName existed named the directory after app.name.
  const packaged = manifest?.dataName ?? manifest?.name;
  if (packaged) return packaged;
  const pkg = JSON.parse(readFileSync(resolve(process.cwd(), "package.json"), "utf8")) as { name?: string };
  if (!pkg.name) throw new Error("nd: could not resolve app name from package.json");
  return pkg.name;
}

function dataRoot(): string {
  switch (process.platform) {
    case "darwin":
      return resolve(homedir(), "Library", "Application Support");
    case "win32":
      return resolve(process.env.APPDATA ?? homedir());
    default:
      return resolve(process.env.XDG_DATA_HOME || resolve(homedir(), ".local", "share"));
  }
}

/** The per-platform user-data directory for this app. Does not create it. */
export function getAppDataDir(): string {
  return join(dataRoot(), appName());
}

/** The Chromium engine's profile root for this app: `cef` inside the app's data
 * directory, unless ND_CEF_CACHE names another. The hosts resolve the same path
 * before Chromium starts (src/cef/app_dir.zig, NDCefEngine.swift), so no two
 * apps share cookies, logins or extensions. Does not create it. */
export function getCefProfileDir(): string {
  return process.env.ND_CEF_CACHE || join(getAppDataDir(), "cef");
}

/** Like `getAppDataDir`, but also creates the directory (recursively) if missing. */
export function ensureAppDataDir(): string {
  const dir = getAppDataDir();
  mkdirSync(dir, { recursive: true });
  return dir;
}
