// Bundle payload assembly: the app-root tree both platform packagers embed
// (Resources/app on mac, AppDir/app on linux). The app root mirrors the
// configured workspaceRoot, so the app's own files land at their
// workspace-relative path and relative imports keep resolving packaged.
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, join, relative, resolve, sep } from "node:path";
import { buildNativePlugins, type NativeDesktopConfig, type PackageConfig } from "../config.ts";
import type { ResolvedIdentity } from "./identity.ts";
import { assertResolvableEntries, flattenRuntimeModules } from "./modules.ts";

export const DEFAULT_ENTRY = "src/main.tsx";

/** The compiled entry: the source entry with its first path segment replaced by the compile outDir. */
export function compiledEntryFor(entry: string, outDir: string): string {
  const segments = entry.split("/");
  if (segments.length < 2) throw new Error(`nd: cannot derive a compiled entry from "${entry}" (set package.compile.entry)`);
  segments[0] = outDir;
  return segments.join("/");
}

/** POSIX-separated path of `to` relative to `from` ("" when equal). Rejects escapes above `from`. */
export function workspaceRelative(from: string, to: string): string {
  const rel = relative(from, to);
  if (rel.startsWith("..")) throw new Error(`nd: app dir ${to} lies outside workspaceRoot ${from}`);
  return rel.split(sep).join("/");
}

export interface PayloadResult {
  /** App-root-relative entry script. */
  entry: string;
  /** App-root-relative working directory for the packaged app. */
  cwd: string;
  /** App-root-relative native plugin paths. */
  pluginPaths: string[];
}

export interface PayloadOptions {
  appDir: string;
  config: NativeDesktopConfig;
  identity: ResolvedIdentity;
  /** The bundle's app root (Resources/app or AppDir/app). */
  appRoot: string;
  /** CLI --entry override (app-relative source entry). */
  entry?: string;
  /** false forces raw source (--no-compile). */
  compile?: boolean;
}

const skipNodeModules = (src: string) => basename(src) !== "node_modules";

/**
 * Copies a payload tree without ever descending into the bundle output
 * (`appRoot`), which nests inside the copied tree under the default config
 * (compile outDir == package outDir == "dist"). cpSync refuses a destination
 * inside its source outright, so that case walks the top level by hand.
 */
function copyPayloadTree(src: string, dest: string, appRoot: string, filter: (p: string) => boolean): void {
  const bundle = resolve(appRoot);
  const containsBundle = (p: string) => {
    const r = resolve(p);
    return r === bundle || bundle.startsWith(r + sep);
  };
  if (!containsBundle(src)) {
    cpSync(src, dest, { recursive: true, dereference: true, filter });
    return;
  }
  mkdirSync(dest, { recursive: true });
  for (const name of readdirSync(src)) {
    const child = join(src, name);
    if (!filter(child) || containsBundle(child)) continue;
    cpSync(child, join(dest, name), { recursive: true, dereference: true, filter });
  }
}

async function runCompile(appDir: string, script: string): Promise<void> {
  const proc = Bun.spawn(["bun", "run", script], {
    cwd: appDir,
    env: process.env,
    stdin: "ignore",
    stdout: "inherit",
    stderr: "inherit",
  });
  if (await proc.exited !== 0) throw new Error(`nd: compile failed (bun run ${script})`);
}

export async function assemblePayload(o: PayloadOptions): Promise<PayloadResult> {
  const pkg = o.config.package ?? {};
  const appDir = resolve(o.appDir);
  const workspaceRoot = resolve(appDir, pkg.workspaceRoot ?? ".");
  const appRel = workspaceRelative(workspaceRoot, appDir);
  const destAppDir = appRel ? join(o.appRoot, appRel) : o.appRoot;
  const sourceEntry = o.entry ?? pkg.entry ?? DEFAULT_ENTRY;

  // Compile step: "auto" runs the app's `compile` script when declared.
  const compileCfg = o.compile === false ? false : pkg.compile ?? "auto";
  const appPkg = JSON.parse(readFileSync(join(appDir, "package.json"), "utf8")) as {
    scripts?: Record<string, string>;
  };
  let entry = sourceEntry;
  if (compileCfg !== false) {
    const script = typeof compileCfg === "object" ? compileCfg.script ?? "compile" : "compile";
    const outDir = typeof compileCfg === "object" ? compileCfg.outDir ?? "dist" : "dist";
    if (typeof compileCfg === "object" || appPkg.scripts?.[script]) {
      await runCompile(appDir, script);
      entry = typeof compileCfg === "object" && compileCfg.entry ? compileCfg.entry : compiledEntryFor(sourceEntry, outDir);
    } else {
      console.error('nd: no "compile" script - packaging raw src/ (react-compiler pass skipped)');
    }
  }

  mkdirSync(destAppDir, { recursive: true });
  const entryDir = entry.includes("/") ? entry.split("/")[0]! : null;
  if (entryDir) {
    copyPayloadTree(join(appDir, entryDir), join(destAppDir, entryDir), o.appRoot, skipNodeModules);
  } else {
    // Root-level entry: ship the whole app dir (minus node_modules and any
    // build output nested inside it, which would recurse into the copy).
    const bundleOut = resolve(workspaceRoot, pkg.outDir ?? "dist");
    const skip = (src: string) => skipNodeModules(src) && resolve(src) !== bundleOut;
    copyPayloadTree(appDir, destAppDir, o.appRoot, skip);
  }
  cpSync(join(appDir, "package.json"), join(destAppDir, "package.json"));

  for (const inc of pkg.include ?? []) {
    const src = join(workspaceRoot, inc);
    if (!existsSync(src)) throw new Error(`nd: package.include path not found: ${src}`);
    cpSync(src, join(o.appRoot, inc), { recursive: true, dereference: true, filter: skipNodeModules });
  }

  const flat = flattenRuntimeModules({
    appDir,
    extraRoots: pkg.runtimeDependencies,
    dest: join(o.appRoot, "node_modules"),
  });
  assertResolvableEntries(o.appRoot, flat);

  // Native plugins: build (or reuse) the app's declared outputs, ship them
  // under app/native/, and record app-root-relative paths for the bootstrap.
  const outputs = await buildNativePlugins(o.config, appDir);
  const pluginPaths: string[] = [];
  for (const output of outputs) {
    const dest = join("native", basename(output));
    mkdirSync(join(o.appRoot, "native"), { recursive: true });
    cpSync(output, join(o.appRoot, dest), { dereference: true });
    pluginPaths.push(dest);
  }

  const cwd = appRel || ".";
  const appEntry = appRel ? `${appRel}/${entry}` : entry;
  writeFileSync(join(o.appRoot, "nd-app.json"), `${JSON.stringify({
    id: o.identity.id,
    name: o.identity.name,
    version: o.identity.version,
    entry: appEntry,
    cwd,
    pluginPaths,
  }, null, 2)}\n`);

  return { entry: appEntry, cwd, pluginPaths };
}
