// CEF acquisition and staging for `nd package`. Nothing here runs unless the
// target's `webview.engine` is "chromium": with the default engine the app ships
// zero Chromium bytes, which is a product claim the doctor's bundle audit
// (cefBundleAudit, below) proves.
//
// Acquisition: the pinned version is resolved against the official build index,
// the "minimal" dist is downloaded, its sha1 from the index is verified, and the
// tarball plus its extraction are cached under ~/.cache/nativedesktop/cef/ so a
// second package run and `nd dev` share one copy.
//
// Staging is split into a pure plan and an executor. The plan is what the tests
// assert, so both platforms' layouts are covered from one cached dist without
// writing a bundle.
import { $ } from "bun";
import { chmodSync, cpSync, existsSync, mkdirSync, readdirSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";
import { buildCefHelper } from "@nativedesktop/host";
import {
  CEF_ARTIFACT_NAMES,
  CEF_HELPER_BINARY_NAME,
  type CefPlatform,
  cefCacheRoot,
  cefDistDir,
  DEFAULT_CEF_VERSION,
  isCefRoot,
} from "@nativedesktop/host/cef";
import type { CefConfig } from "../config.ts";
import { cefHelperEntitlements, cefHelperPlist } from "./templates.ts";

const CEF_INDEX_URL = "https://cef-builds.spotifycdn.com/index.json";
const CEF_DOWNLOAD_BASE = "https://cef-builds.spotifycdn.com/";

/** Locales staged when the config names none. */
export const DEFAULT_CEF_LOCALES: readonly string[] = ["en-US"];

export { CEF_HELPER_BINARY_NAME } from "@nativedesktop/host/cef";

/** Helper bundle suffixes CEF requires on macOS, in the order they are signed.
 * The names are fixed by CEF: it derives each helper path from the main bundle
 * name plus exactly these suffixes. */
export const CEF_HELPER_SUFFIXES: readonly string[] = ["", " (Alerts)", " (GPU)", " (Plugin)", " (Renderer)"];

/** Suffix to bundle-id tail, e.g. " (Renderer)" to ".renderer". */
function helperIdSuffix(suffix: string): string {
  return suffix ? `.${suffix.trim().replace(/[()]/g, "").toLowerCase()}` : "";
}

// ---------------------------------------------------------------------------
// Acquisition
// ---------------------------------------------------------------------------

interface CefIndexFile {
  type: string;
  name: string;
  sha1: string;
  size: number;
}

interface CefIndexVersion {
  cef_version: string;
  channel: string;
  files: CefIndexFile[];
}

export interface CefIndex {
  [platform: string]: { versions: CefIndexVersion[] };
}

export interface CefBuild {
  /** Full version as the index spells it, e.g. "151.3.23+gd211df0+chromium-151.0.7922.170". */
  cefVersion: string;
  name: string;
  sha1: string;
  size: number;
  url: string;
}

/**
 * The minimal dist for a pinned version. The pin is the short release number
 * ("151.3.23"); the index keys on the full build string, so it matches on the
 * "+" boundary rather than a bare prefix (151.3.2 must not match 151.3.23).
 */
export function selectCefBuild(index: CefIndex, cefPlatform: CefPlatform, version: string): CefBuild {
  const versions = index[cefPlatform]?.versions;
  if (!versions?.length) throw new Error(`nd: the CEF build index lists no builds for "${cefPlatform}"`);
  const entry = versions.find((v) => v.cef_version === version || v.cef_version.startsWith(`${version}+`));
  if (!entry) {
    throw new Error(`nd: CEF ${version} has no ${cefPlatform} build in the index (newest is ${versions[0]!.cef_version})`);
  }
  const file = entry.files.find((f) => f.type === "minimal");
  if (!file) throw new Error(`nd: CEF ${entry.cef_version} ships no minimal ${cefPlatform} dist`);
  return {
    cefVersion: entry.cef_version,
    name: file.name,
    sha1: file.sha1,
    size: file.size,
    url: CEF_DOWNLOAD_BASE + encodeURIComponent(file.name),
  };
}

/** The build index. ND_CEF_INDEX points at a local copy (offline builds, tests). */
export async function fetchCefIndex(env: Record<string, string | undefined> = process.env): Promise<CefIndex> {
  const local = env.ND_CEF_INDEX;
  if (local) return (await Bun.file(local).json()) as CefIndex;
  const response = await fetch(CEF_INDEX_URL);
  if (!response.ok) throw new Error(`nd: CEF build index request failed (${response.status} ${response.statusText})`);
  return (await response.json()) as CefIndex;
}

export async function sha1OfFile(path: string): Promise<string> {
  const hasher = new Bun.CryptoHasher("sha1");
  for await (const chunk of Bun.file(path).stream()) hasher.update(chunk);
  return hasher.digest("hex");
}

export interface EnsureCefDistOptions {
  cefPlatform: CefPlatform;
  version?: string;
  env?: Record<string, string | undefined>;
  /** Refuse to touch the network: the dist must already be cached. */
  offline?: boolean;
}

/**
 * Absolute path to an extracted dist, downloading and verifying it first when
 * the cache has none. An extraction counts as complete only once the platform's
 * loadable is in place, so a killed run re-extracts instead of staging a
 * half-written tree.
 */
export async function ensureCefDist(opts: EnsureCefDistOptions): Promise<string> {
  const env = opts.env ?? process.env;
  const version = opts.version ?? DEFAULT_CEF_VERSION;
  const dist = cefDistDir(version, opts.cefPlatform, env);
  if (isCefRoot(join(dist, "Release"), opts.cefPlatform)) return dist;
  if (opts.offline) {
    throw new Error(`nd: CEF ${version} for ${opts.cefPlatform} is not in the cache (${dist}) and downloads are disabled`);
  }

  const build = selectCefBuild(await fetchCefIndex(env), opts.cefPlatform, version);
  const cache = cefCacheRoot(env);
  mkdirSync(cache, { recursive: true });
  const tarball = join(cache, `cef_${version}-${opts.cefPlatform}_minimal.tar.bz2`);
  if (!existsSync(tarball) || (await sha1OfFile(tarball)) !== build.sha1) {
    console.error(`nd: downloading CEF ${build.cefVersion} for ${opts.cefPlatform} (${(build.size / 1e6).toFixed(0)} MB)`);
    const response = await fetch(build.url);
    if (!response.ok) throw new Error(`nd: CEF download failed (${response.status} ${response.statusText}) for ${build.url}`);
    await Bun.write(tarball, response);
    const got = await sha1OfFile(tarball);
    if (got !== build.sha1) {
      rmSync(tarball, { force: true });
      throw new Error(`nd: CEF download sha1 mismatch for ${build.name} (index ${build.sha1}, got ${got})`);
    }
  }

  // Extract beside the target and rename, so an interrupted run cannot leave a
  // partial tree that later passes the completeness check. --strip-components
  // drops the tarball's cef_binary_<version>_<platform>_minimal/ wrapper, and
  // tar keeps symlinks as symlinks, which the macOS framework layout needs for
  // codesign to accept it.
  const staging = `${dist}.partial`;
  rmSync(staging, { recursive: true, force: true });
  mkdirSync(staging, { recursive: true });
  await $`tar -xjf ${tarball} -C ${staging} --strip-components=1`;
  rmSync(dist, { recursive: true, force: true });
  renameSync(staging, dist);
  return dist;
}

// ---------------------------------------------------------------------------
// macOS staging
// ---------------------------------------------------------------------------

export interface CefHelperPlan {
  /** "" for the base helper, " (Renderer)" and friends for the rest. */
  suffix: string;
  appPath: string;
  executablePath: string;
  bundleId: string;
  plist: string;
  /** Renderer and GPU run JIT-compiled code and need the entitlements for it. */
  jit: boolean;
}

export interface CefMacPlan {
  framework: { from: string; to: string };
  /** Mach-O inside the framework that codesign has to seal before the bundle. */
  frameworkLibraries: string[];
  helpers: CefHelperPlan[];
  helperBinary: string;
}

export interface CefMacPlanInput {
  distRoot: string;
  /** The .app's Contents directory. */
  contents: string;
  appName: string;
  appId: string;
  version: string;
  minimumSystemVersion: string;
  helperBinary: string;
}

const FRAMEWORK_NAME = "Chromium Embedded Framework.framework";

/** What `nd package mac` will put in Contents/Frameworks, without writing it. */
export function planCefMac(input: CefMacPlanInput): CefMacPlan {
  const from = join(input.distRoot, "Release", FRAMEWORK_NAME);
  if (!existsSync(join(from, "Chromium Embedded Framework"))) {
    throw new Error(`nd: the CEF dist at ${input.distRoot} has no ${FRAMEWORK_NAME}`);
  }
  const frameworks = join(input.contents, "Frameworks");
  const to = join(frameworks, FRAMEWORK_NAME);
  const librariesDir = join(from, "Libraries");
  const frameworkLibraries = existsSync(librariesDir)
    ? readdirSync(librariesDir).filter((f) => f.endsWith(".dylib")).sort().map((f) => join(to, "Libraries", f))
    : [];

  const helpers = CEF_HELPER_SUFFIXES.map((suffix) => {
    const name = `${input.appName} Helper${suffix}`;
    const appPath = join(frameworks, `${name}.app`);
    const bundleId = `${input.appId}.helper${helperIdSuffix(suffix)}`;
    return {
      suffix,
      appPath,
      executablePath: join(appPath, "Contents", "MacOS", name),
      bundleId,
      plist: cefHelperPlist({
        name,
        id: bundleId,
        version: input.version,
        minimumSystemVersion: input.minimumSystemVersion,
      }),
      jit: suffix === " (Renderer)" || suffix === " (GPU)",
    };
  });

  return { framework: { from, to }, frameworkLibraries, helpers, helperBinary: input.helperBinary };
}

/** Inside-out codesign order for the staged CEF payload: the framework's own
 * Mach-O, the framework, then each helper executable and its bundle. Everything
 * here is signed before the outer .app. */
export function cefMacSignTargets(plan: CefMacPlan): { path: string; jit: boolean }[] {
  const targets = plan.frameworkLibraries.map((path) => ({ path, jit: false }));
  targets.push({ path: plan.framework.to, jit: false });
  for (const helper of plan.helpers) {
    targets.push({ path: helper.executablePath, jit: helper.jit });
    targets.push({ path: helper.appPath, jit: helper.jit });
  }
  return targets;
}

/** Writes a mac plan: the framework tree and the five helper bundles. */
export function applyCefMacPlan(plan: CefMacPlan): void {
  if (!existsSync(plan.helperBinary)) {
    throw new Error(
      `nd: the CEF helper executable is missing (${plan.helperBinary}). ` +
        `engine "chromium" needs ${CEF_HELPER_BINARY_NAME} from the AppKit host build; ` +
        `point ND_CEF_HELPER at one, or package with engine "system".`,
    );
  }
  mkdirSync(join(plan.framework.to, ".."), { recursive: true });
  rmSync(plan.framework.to, { recursive: true, force: true });
  // verbatimSymlinks keeps the framework's internal links as links. Resolving
  // them duplicates the 224 MB binary and makes codesign reject the bundle.
  cpSync(plan.framework.from, plan.framework.to, { recursive: true, verbatimSymlinks: true });
  for (const helper of plan.helpers) {
    mkdirSync(join(helper.appPath, "Contents", "MacOS"), { recursive: true });
    cpSync(plan.helperBinary, helper.executablePath, { dereference: true });
    chmodSync(helper.executablePath, 0o755);
    writeFileSync(join(helper.appPath, "Contents", "Info.plist"), helper.plist);
  }
}

/** Entitlements file for the JIT helpers, written next to the app's own. */
export function writeCefHelperEntitlements(dir: string): string {
  const path = join(dir, "cef-helper-entitlements.plist");
  writeFileSync(path, cefHelperEntitlements());
  return path;
}

// ---------------------------------------------------------------------------
// Linux staging
// ---------------------------------------------------------------------------

/** Release/ payload, from the dist's own cmake/cef_variables.cmake list.
 * libvulkan.so.1 belongs to the swiftshader set: dropping it leaves the
 * software renderer unable to load. */
const LINUX_BINARIES: readonly string[] = [
  "libcef.so",
  "libEGL.so",
  "libGLESv2.so",
  "libvk_swiftshader.so",
  "libvulkan.so.1",
  "vk_swiftshader_icd.json",
  "v8_context_snapshot.bin",
  "chrome-sandbox",
];

/** Resources/ payload, minus locales/, which is trimmed to the config list. */
const LINUX_RESOURCES: readonly string[] = [
  "icudtl.dat",
  "resources.pak",
  "chrome_100_percent.pak",
  "chrome_200_percent.pak",
];

export interface CefLinuxFile {
  from: string;
  to: string;
  /** libcef.so ships with full debug info: 1.43 GB unstripped, ~269 MB stripped. */
  strip?: boolean;
}

export interface CefLinuxPlan {
  root: string;
  files: CefLinuxFile[];
  locales: CefLinuxFile[];
  /** Dist entries the plan wanted and could not find. A non-empty list is fatal. */
  missing: string[];
}

export interface CefLinuxPlanInput {
  distRoot: string;
  /** The AppDir root. */
  appDir: string;
  locales?: string[];
}

/** What `nd package linux` will put in <AppDir>/lib/cef, without writing it. */
export function planCefLinux(input: CefLinuxPlanInput): CefLinuxPlan {
  const root = join(input.appDir, "lib", "cef");
  const release = join(input.distRoot, "Release");
  const resources = join(input.distRoot, "Resources");
  const missing: string[] = [];
  const files: CefLinuxFile[] = [];

  for (const name of LINUX_BINARIES) {
    const from = join(release, name);
    if (!existsSync(from)) missing.push(`Release/${name}`);
    files.push({ from, to: join(root, name), strip: name === "libcef.so" });
  }
  for (const name of LINUX_RESOURCES) {
    const from = join(resources, name);
    if (!existsSync(from)) missing.push(`Resources/${name}`);
    files.push({ from, to: join(root, name) });
  }

  const wanted = input.locales?.length ? input.locales : DEFAULT_CEF_LOCALES;
  const locales = wanted.map((locale) => {
    const from = join(resources, "locales", `${locale}.pak`);
    if (!existsSync(from)) missing.push(`Resources/locales/${locale}.pak`);
    return { from, to: join(root, "locales", `${locale}.pak`) };
  });

  return { root, files, locales, missing };
}

/** Writes a linux plan, preserving each file's mode from the dist. chrome-sandbox
 * ships mode 0755 and needs 4755 root-owned to use the setuid sandbox; that is a
 * post-install step for whoever installs the AppImage, not something a packaging
 * run can do for them (see the doctor's cef note). The userns sandbox is the
 * automatic fallback, so `--no-sandbox` is never the answer. */
export async function applyCefLinuxPlan(plan: CefLinuxPlan): Promise<void> {
  if (plan.missing.length) {
    throw new Error(`nd: the CEF dist is missing ${plan.missing.length} staged file(s): ${plan.missing.join(", ")}`);
  }
  mkdirSync(join(plan.root, "locales"), { recursive: true });
  for (const file of [...plan.files, ...plan.locales]) {
    cpSync(file.from, file.to, { dereference: true });
    chmodSync(file.to, statSync(file.from).mode & 0o7777);
  }
  const strip = plan.files.find((f) => f.strip);
  if (strip) await stripInPlace(strip.to);
}

/** `strip` on the staged libcef.so. Cross-packaging from macOS has no ELF strip,
 * so a failure warns and ships the unstripped copy rather than failing the run. */
async function stripInPlace(path: string): Promise<void> {
  const before = statSync(path).size;
  const result = await $`strip ${path}`.quiet().nothrow();
  if (result.exitCode !== 0) {
    console.error(`ND_WARN nd: strip failed on ${basename(path)}; shipping it unstripped (${(before / 1e9).toFixed(2)} GB)`);
    return;
  }
  const after = statSync(path).size;
  console.error(`ND_PACKAGE_CEF_STRIP ${basename(path)} ${(before / 1e6).toFixed(0)}MB -> ${(after / 1e6).toFixed(0)}MB`);
}

// ---------------------------------------------------------------------------
// The zero-bytes claim
// ---------------------------------------------------------------------------

/**
 * Every CEF artifact found in a bundle, bundle-relative. An engine="system"
 * build must return an empty list: that is the "zero Chromium bytes unless the
 * config asks for it" claim, checked rather than asserted.
 */
export function cefBundleAudit(bundleRoot: string): string[] {
  const names = new Set(CEF_ARTIFACT_NAMES);
  const found: string[] = [];
  const walk = (dir: string, prefix: string): void => {
    let entries;
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (names.has(entry.name)) {
        found.push(rel);
        continue;
      }
      if (entry.isDirectory() && !entry.isSymbolicLink()) walk(join(dir, entry.name), rel);
    }
  };
  walk(bundleRoot, "");
  return found.sort();
}

/** The pinned CEF release for a config. */
export function cefVersionFor(cef: CefConfig | undefined): string {
  return cef?.version ?? DEFAULT_CEF_VERSION;
}

/** The helper executable: ND_CEF_HELPER, else beside the host binary. */
export function resolveCefHelperBinary(
  hostBinary: string,
  env: Record<string, string | undefined> = process.env,
): string {
  return env.ND_CEF_HELPER ?? join(hostBinary, "..", CEF_HELPER_BINARY_NAME);
}

/** The helper executable, built from the framework checkout when the host is a
 * source build and the helper product has never been built. Same resolution the
 * host binary itself gets: use what is there, otherwise build it, and only then
 * give up. */
export async function ensureCefHelperBinary(
  hostBinary: string,
  env: Record<string, string | undefined> = process.env,
): Promise<string> {
  const path = resolveCefHelperBinary(hostBinary, env);
  if (existsSync(path)) return path;
  if (!env.ND_CEF_HELPER) {
    const built = await buildCefHelper(hostBinary);
    if (built) return built;
  }
  throw new Error(
    `nd: the CEF helper executable is missing (${path}). ` +
      `engine "chromium" needs ${CEF_HELPER_BINARY_NAME} from the AppKit host build; ` +
      "point ND_CEF_HELPER at one, or package with engine \"system\".",
  );
}
