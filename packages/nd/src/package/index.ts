// packageApp(): the `nd package` entry point ND's own tools call. Loads the
// app's nativedesktop.config.ts, resolves its identity, cleans the platform
// output dir, and dispatches to the platform packager. mac.ts is dynamically
// imported so linux legs never load it.
import { rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { loadConfig } from "../config.ts";
import { resolveIdentity } from "./identity.ts";

export interface PackageOptions {
  platform: "mac" | "linux";
  /** App directory (where nativedesktop.config.ts + package.json live). Default: process.cwd(). */
  cwd?: string;
  /** Output root override, resolved against process.cwd(). Default: package.outDir against workspaceRoot. */
  outDir?: string;
  /** App-relative source entry override. */
  entry?: string;
  version?: string;
  /** false forces raw source (--no-compile). */
  compile?: boolean;
  /** string: codesign identity; null: skip signing; undefined: config/env/ad-hoc. */
  signIdentity?: string | null;
  notarize?: boolean;
  format?: "appimage" | "appdir";
}

export interface PackageResult {
  bundlePath: string;
  updateManifest?: string;
  publicKey?: string;
}

export async function packageApp(o: PackageOptions): Promise<PackageResult> {
  const appDir = resolve(process.cwd(), o.cwd ?? ".");
  const config = await loadConfig(appDir);
  const identity = resolveIdentity(config, appDir, { version: o.version });
  const workspaceRoot = resolve(appDir, config.package?.workspaceRoot ?? ".");
  const outDir = o.outDir ? resolve(process.cwd(), o.outDir) : resolve(workspaceRoot, config.package?.outDir ?? "dist");

  // Assembly is not idempotent (cpSync throws on a leftover tree): always
  // start the platform dir from a clean slate.
  rmSync(join(outDir, o.platform), { recursive: true, force: true });

  if (o.platform === "mac") {
    // Dynamic import so the linux path never loads AppKit-side packaging.
    const { packageMacApp } = await import("./mac.ts");
    return packageMacApp(appDir, config, identity, outDir, o);
  }
  const { packageLinuxApp } = await import("./linux.ts");
  return packageLinuxApp(appDir, config, identity, outDir, o);
}
