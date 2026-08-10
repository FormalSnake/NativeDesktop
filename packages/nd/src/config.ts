import { existsSync, statSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { pathToFileURL } from "node:url";

export interface NativeBuildCommand {
  command: string[];
  /** Files/directories whose mtimes invalidate the cached output. */
  inputs?: string[];
}

export interface NativePluginConfig {
  /** Shared library path, or a per-platform path. */
  path?: string;
  darwin?: string;
  linux?: string;
  build?: NativeBuildCommand | Partial<Record<"darwin" | "linux", NativeBuildCommand>>;
}

export interface FileAssociation {
  /** Extension without the dot, e.g. "md". */
  ext: string;
  /** Human-readable document type name. */
  name?: string;
  /** e.g. "text/markdown" — drives the Linux .desktop MimeType entry. */
  mimeType?: string;
  /** macOS CFBundleTypeRole. Defaults to "editor". */
  role?: "editor" | "viewer";
}

export interface UrlScheme {
  scheme: string;
  name?: string;
}

export interface AppIcon {
  /** Shared icon source (.png/.svg/.icns path, app-relative). */
  source?: string;
  macos?: string;
  linux?: string;
}

export interface AppIdentity {
  /** Reverse-DNS bundle id. Required for icons, mime registration, and updates. */
  id?: string;
  /** Product name: <Name>.app, usr/bin/<slug>, AppImage basename. Default: package.json name. */
  name?: string;
  /** CFBundleDisplayName / .desktop Name. Default: name. */
  displayName?: string;
  /** Default: package.json version, then "0.0.0". ND_APP_VERSION and --version override. */
  version?: string;
  icon?: string | AppIcon;
  /** .desktop Categories=. Default ["Utility"]. */
  categories?: string[];
  fileAssociations?: FileAssociation[];
  urlSchemes?: UrlScheme[];
}

export interface UpdatesConfig {
  baseUrl: string;
  /** minisign secret key path. Falls back to ND_MINISIGN_SEC/ND_MINISIGN_PUB. */
  secretKey?: string;
  publicKey?: string;
  /** Generate a throwaway keypair when no key is configured (CI/test path only). */
  ephemeralKey?: boolean;
  format?: "tar.gz" | "tar.zst";
}

export interface MacPackageConfig {
  /** Info.plist override file path; the default template is generated in-code. */
  infoPlist?: string;
  entitlements?: string;
  /** Default "26.0". */
  minimumSystemVersion?: string;
  /** LSApplicationCategoryType. */
  category?: string;
  /** Else APPLE_SIGN_IDENTITY, else ad-hoc "-". */
  signIdentity?: string;
  /** Default: auto (only with all three Apple creds). */
  notarize?: boolean;
  extraPlist?: Record<string, string | number | boolean>;
  backend?: "appkit" | "gtk";
}

export interface LinuxPackageConfig {
  /** AppDir seed tree copied before generated files land. */
  appDirTemplate?: string;
  format?: "appimage" | "appdir";
  /** Extra/overriding .desktop entry lines. */
  desktopEntry?: Record<string, string>;
}

export interface PackageConfig {
  /** Source entry, app-relative. Default "src/main.tsx". */
  entry?: string;
  /** Default "auto": run the app's `compile` script when it declares one. */
  compile?: "auto" | false | { script?: string; outDir?: string; entry?: string };
  /** App-relative, e.g. "../..". The bundle app root mirrors this directory. Default: the app dir itself. */
  workspaceRoot?: string;
  /** Extra workspace-relative dirs/files copied into the bundle app root. */
  include?: string[];
  /** Extra roots for the runtime module-graph walk. */
  runtimeDependencies?: string[];
  /** Default "dist" (resolved against workspaceRoot). */
  outDir?: string;
  bunPath?: string;
  mac?: MacPackageConfig;
  linux?: LinuxPackageConfig;
  updates?: UpdatesConfig;
}

export interface NativeDesktopConfig {
  native?: { plugins?: NativePluginConfig[] };
  app?: AppIdentity;
  package?: PackageConfig;
}

export function defineConfig(config: NativeDesktopConfig): NativeDesktopConfig { return config; }

export async function loadConfig(cwd = process.cwd()): Promise<NativeDesktopConfig> {
  const path = resolve(cwd, "nativedesktop.config.ts");
  if (!existsSync(path)) return {};
  const mod = await import(pathToFileURL(path).href);
  return (mod.default ?? {}) as NativeDesktopConfig;
}

export async function buildNativePlugins(config: NativeDesktopConfig, cwd = process.cwd()): Promise<string[]> {
  const platform = process.platform;
  if (platform !== "darwin" && platform !== "linux") return [];
  const jobs: { output: string; build?: NativeBuildCommand }[] = [];
  for (const plugin of config.native?.plugins ?? []) {
    const declared = plugin[platform] ?? plugin.path;
    if (!declared) {
      if (!plugin.path && !plugin.darwin && !plugin.linux) throw new Error("nd: native plugin requires path, darwin, or linux output");
      console.error(`nd: skipping native plugin (no ${platform} output)`);
      continue;
    }
    const output = resolve(cwd, declared);
    const configured = plugin.build;
    const build = configured && "command" in configured
      ? configured as NativeBuildCommand
      : (configured as Partial<Record<"darwin" | "linux", NativeBuildCommand>> | undefined)?.[platform];
    const stale = build && needsBuild(output, ["nativedesktop.config.ts", ...(build.inputs ?? [])], cwd);
    jobs.push({ output, build: stale ? build : undefined });
  }
  const pending = jobs.filter((job) => job.build);
  if (pending.length) {
    const env = buildCommandEnv(cwd, platform);
    await Promise.all(pending.map(async ({ build }) => {
      const proc = Bun.spawn(build!.command, { cwd, env, stdin: "ignore", stdout: "pipe", stderr: "pipe" });
      const [out, err, status] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text(), proc.exited]);
      if (out) process.stdout.write(out);
      if (err) process.stderr.write(err);
      if (status !== 0) throw new Error(`nd: native build failed (${build!.command.join(" ")})`);
    }));
  }
  const outputs: string[] = [];
  for (const { output } of jobs) {
    if (!existsSync(output)) throw new Error(`nd: native plugin output not found: ${output}`);
    outputs.push(output);
  }
  return outputs;
}

/** Env for plugin build commands: on darwin, scrub Nix/SDK overrides that break
 * xcrun/swiftc and pin DEVELOPER_DIR to the selected Xcode; everywhere, expose
 * the installed @nativedesktop/native package root as ND_NATIVE_PACKAGE. */
function buildCommandEnv(cwd: string, platform: "darwin" | "linux"): Record<string, string | undefined> {
  const env: Record<string, string | undefined> = { ...process.env };
  if (platform === "darwin") {
    delete env.SDKROOT;
    delete env.DEVELOPER_SDK_DIR;
    delete env.NIX_CFLAGS_COMPILE;
    delete env.NIX_LDFLAGS;
    // xcode-select -p echoes an inherited DEVELOPER_DIR back, so query with it
    // cleared to read the machine's actual selection.
    const xcode = Bun.spawnSync(["xcode-select", "-p"], { env: { ...env, DEVELOPER_DIR: undefined } });
    if (xcode.exitCode === 0) env.DEVELOPER_DIR = xcode.stdout.toString().trim();
  }
  try {
    env.ND_NATIVE_PACKAGE = dirname(Bun.resolveSync("@nativedesktop/native/package.json", cwd));
  } catch {
    // App has no @nativedesktop/native dependency — leave the variable unset.
  }
  return env;
}

function needsBuild(output: string, inputs: string[], cwd: string): boolean {
  const resolved = inputs.map((input) => resolve(cwd, input));
  for (const input of resolved) {
    if (!existsSync(input)) throw new Error(`nd: native build input not found: ${input}`);
  }
  if (!existsSync(output)) return true;
  const outputTime = statSync(output).mtimeMs;
  return resolved.some((input) => newestMtime(input) > outputTime);
}

function newestMtime(path: string): number {
  const stat = statSync(path);
  if (!stat.isDirectory()) return stat.mtimeMs;
  const glob = new Bun.Glob("**/*");
  let newest = stat.mtimeMs;
  for (const relative of glob.scanSync({ cwd: path, onlyFiles: true })) newest = Math.max(newest, statSync(resolve(path, relative)).mtimeMs);
  return newest;
}
