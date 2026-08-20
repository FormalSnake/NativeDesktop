// @nativedesktop/host/cef: where the Chromium Embedded Framework lives for a
// given platform. CEF is never linked into a host binary (it is dlopened at
// runtime), so `nd dev`, `nd package`, `nd doctor` and both engine backends all
// have to agree on one search order. This module is that order for the
// TypeScript side; the Zig and Swift loaders read the same three places in the
// same sequence.
//
//   1. ND_CEF_ROOT       an explicit directory, ahead of everything else
//   2. the app bundle    Contents/Frameworks (mac), lib/cef (linux)
//   3. the dev cache     ~/.cache/nativedesktop/cef/<version>-<platform>/Release
//
// Nothing here downloads anything: resolution answers from what is already on
// disk. Fetching and staging a dist is `nd package`'s job (packages/nd).
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";

/** CEF release the framework pins. `webview.cef.version` overrides it. */
export const DEFAULT_CEF_VERSION = "151.3.23";

/** Platform names as cef-builds.spotifycdn.com spells them. */
export type CefPlatform = "macosarm64" | "macosx64" | "linux64" | "linuxarm64";

const CEF_PLATFORMS: Record<string, CefPlatform> = {
  "darwin-arm64": "macosarm64",
  "darwin-x64": "macosx64",
  "linux-x64": "linux64",
  "linux-arm64": "linuxarm64",
};

/** The CEF platform key for an os/arch pair, e.g. "macosarm64". */
export function cefPlatformKey(platform: string = process.platform, arch: string = process.arch): CefPlatform {
  const key = CEF_PLATFORMS[`${platform}-${arch}`];
  if (!key) throw new Error(`@nativedesktop/host: no CEF build for "${platform}-${arch}"`);
  return key;
}

/** True for the two macOS keys, which carry a framework instead of a .so. */
export function isMacCefPlatform(cefPlatform: CefPlatform): boolean {
  return cefPlatform.startsWith("macos");
}

/** The loadable, relative to the directory that holds it. Its presence is what
 * makes a candidate directory a CEF root. */
export function cefLoadableName(cefPlatform: CefPlatform): string {
  return isMacCefPlatform(cefPlatform)
    ? "Chromium Embedded Framework.framework/Chromium Embedded Framework"
    : "libcef.so";
}

/** ~/.cache/nativedesktop/cef, or $XDG_CACHE_HOME/nativedesktop/cef. */
export function cefCacheRoot(env: Record<string, string | undefined> = process.env): string {
  const base = env.XDG_CACHE_HOME || join(env.HOME || homedir(), ".cache");
  return join(base, "nativedesktop", "cef");
}

/** Extracted dist for a version+platform: <cache>/<version>-<platform>, the
 * tarball's contents with its top-level cef_binary_… directory stripped. */
export function cefDistDir(
  version: string,
  cefPlatform: CefPlatform,
  env: Record<string, string | undefined> = process.env,
): string {
  return join(cefCacheRoot(env), `${version}-${cefPlatform}`);
}

export interface CefRootOptions {
  cefPlatform: CefPlatform;
  version?: string;
  /** A packaged bundle to search: the .app on macOS, the AppDir on Linux. */
  bundleRoot?: string;
  env?: Record<string, string | undefined>;
}

/** The three candidate directories, in resolution order, whether or not they exist. */
export function cefRootCandidates(opts: CefRootOptions): string[] {
  const env = opts.env ?? process.env;
  const version = opts.version ?? DEFAULT_CEF_VERSION;
  const candidates: string[] = [];
  if (env.ND_CEF_ROOT) candidates.push(resolve(env.ND_CEF_ROOT));
  if (opts.bundleRoot) candidates.push(bundleCefDir(opts.bundleRoot, opts.cefPlatform));
  candidates.push(join(cefDistDir(version, opts.cefPlatform, env), "Release"));
  return candidates;
}

/** Where a packaged bundle keeps its CEF payload. */
export function bundleCefDir(bundleRoot: string, cefPlatform: CefPlatform): string {
  return isMacCefPlatform(cefPlatform)
    ? join(bundleRoot, "Contents", "Frameworks")
    : join(bundleRoot, "lib", "cef");
}

/** A directory counts as a CEF root once it holds the platform's loadable. */
export function isCefRoot(dir: string, cefPlatform: CefPlatform): boolean {
  return existsSync(join(dir, cefLoadableName(cefPlatform)));
}

/** First candidate that actually holds the loadable, or undefined. */
export function resolveCefRoot(opts: CefRootOptions): string | undefined {
  return cefRootCandidates(opts).find((dir) => isCefRoot(dir, opts.cefPlatform));
}

/**
 * Basenames of the CEF artifacts a bundle would carry. The doctor check that
 * proves an engine="system" bundle ships zero Chromium bytes matches on these,
 * so one list covers both platforms' layouts.
 */
export const CEF_ARTIFACT_NAMES: readonly string[] = [
  "Chromium Embedded Framework.framework",
  "libcef.so",
  "chrome-sandbox",
  "icudtl.dat",
  "resources.pak",
  "chrome_100_percent.pak",
  "chrome_200_percent.pak",
  "v8_context_snapshot.bin",
  "libvk_swiftshader.so",
  "vk_swiftshader_icd.json",
];
