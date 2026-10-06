// Per-app user-data directory, following each OS's native convention: the
// equivalent of Electron's `app.getPath('userData')`. The directory is named
// after the app's package.json `name` in dev and packaged runs alike (a packaged
// app finds it in the bundle's nd-app.json, written by `nd package` and found by
// walking up from cwd), so both modes share one profile.

import { existsSync, mkdirSync, readFileSync, renameSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";

interface BundleManifest {
  name?: string;
  dataName?: string;
  previousName?: string;
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

function appNames(): { name: string; previous?: string } {
  const manifest = bundleManifest();
  // Bundles packaged before dataName existed named the directory after app.name.
  const packaged = manifest?.dataName ?? manifest?.name;
  if (packaged) return { name: packaged, previous: manifest?.previousName };
  const pkg = JSON.parse(readFileSync(resolve(process.cwd(), "package.json"), "utf8")) as { name?: string };
  if (!pkg.name) throw new Error("nd: could not resolve app name from package.json");
  return { name: pkg.name, previous: process.env.ND_APP_PREVIOUS_NAME || undefined };
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

/** The per-platform user-data directory for this app. Does not create it.
 * After a rename (`app.previousName`), the first call moves the old directory
 * here, once, when this one does not exist yet. */
export function getAppDataDir(): string {
  const { name, previous } = appNames();
  const root = dataRoot();
  const dir = join(root, name);
  if (previous && !existsSync(dir)) {
    // Packaged runs used the old app name, dev runs the old package name,
    // which is normally its lowercase slug.
    const slug = previous.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
    const old = [previous, slug].map((candidate) => join(root, candidate)).find((path) => existsSync(path));
    if (old) renameSync(old, dir);
  }
  return dir;
}

/** Like `getAppDataDir`, but also creates the directory (recursively) if missing. */
export function ensureAppDataDir(): string {
  const dir = getAppDataDir();
  mkdirSync(dir, { recursive: true });
  return dir;
}
