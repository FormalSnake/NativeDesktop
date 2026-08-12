// @nativedesktop/host: resolves the host binary that draws the app for the
// current platform. Two backends exist: the GTK/Zig host (`nd-hello`, native on
// Linux, GTK-via-Quartz on macOS) and the AppKit/SwiftPM host (`nd-shell`,
// macOS-only). The default follows the platform (appkit on darwin, gtk on
// linux), overridable by an explicit `{ backend }` option or the
// ND_BACKEND env var.
//
// How binaries get resolved:
//   - published install: the binary ships in a per-platform package
//     (@nativedesktop/host-darwin-arm64, @nativedesktop/host-linux-x64) listed
//     as optionalDependencies of this package, the Electron/esbuild model. npm
//     and bun install only the package matching the machine's os/cpu.
//   - source-checkout fallback: when this package sits inside the NativeDesktop
//     repo, a missing prebuilt falls back to the freshly built zig-out / swift
//     .build artifacts, and if those are missing too the requested backend is
//     built on first run.
import { existsSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";

const OS_NAMES: Record<string, string> = { darwin: "darwin", linux: "linux", win32: "windows" };
const ARCH_NAMES: Record<string, string> = { arm64: "arm64", x64: "x64" };

export type Backend = "gtk" | "appkit";

/** Host binary basename per backend. */
const BINARY_NAMES: Record<Backend, string> = { gtk: "nd-hello", appkit: "nd-shell" };

/** Platform packages that ship a prebuilt host binary, keyed backend:os-arch.
 * gtk-on-macOS has no entry by design: the GTK host links Homebrew paths and
 * is a source-checkout-only dev path there. */
const PLATFORM_PACKAGES: Record<string, string> = {
  "appkit:darwin-arm64": "@nativedesktop/host-darwin-arm64",
  "gtk:linux-x64": "@nativedesktop/host-linux-x64",
};

/** `<os>-<arch>` key, e.g. "darwin-arm64". */
export function hostPlatformKey(platform: string = process.platform, arch: string = process.arch): string {
  const os = OS_NAMES[platform];
  const cpu = ARCH_NAMES[arch];
  if (!os || !cpu) {
    throw new Error(`@nativedesktop/host: unsupported platform "${platform}-${arch}"`);
  }
  return `${os}-${cpu}`;
}

/** The npm package carrying the prebuilt binary for a backend on a platform
 * key, or undefined when no prebuilt exists for that combination. */
export function hostPackageName(backend: Backend, key: string): string | undefined {
  return PLATFORM_PACKAGES[`${backend}:${key}`];
}

/** Default backend for a platform: appkit ships on macOS, gtk everywhere else. */
function defaultBackend(platform: string): Backend {
  return platform === "darwin" ? "appkit" : "gtk";
}

/**
 * Resolve the requested backend from an explicit option, then ND_BACKEND, then
 * the platform default. The appkit backend is macOS-only; requesting it
 * elsewhere is a hard error rather than a silent GTK fallback.
 */
export function resolveBackend(
  opts: { backend?: Backend } = {},
  env: Record<string, string | undefined> = process.env,
  platform: string = process.platform,
): Backend {
  const requested = opts.backend ?? (env.ND_BACKEND as string | undefined) ?? defaultBackend(platform);
  if (requested !== "gtk" && requested !== "appkit") {
    throw new Error(`@nativedesktop/host: unknown backend "${requested}" (expected "gtk" or "appkit")`);
  }
  if (requested === "appkit" && platform !== "darwin") {
    throw new Error(
      `@nativedesktop/host: the appkit backend is macOS-only (requested on "${platform}"). ` +
        `Use the gtk backend, which runs natively on Linux.`,
    );
  }
  return requested;
}

interface Candidates {
  /** Platform package expected to carry the prebuilt binary, if one exists. */
  packageName: string | undefined;
  /** Binary basename inside the platform package's bin/ (with .exe on win32). */
  binaryName: string;
  /** Repo root two levels up from the package, for the source-checkout fallback. */
  repoRoot: string;
  /** Freshly built artifacts, in preference order. */
  fresh: string[];
}

/** Pure path computation for a backend on a given platform, the resolution matrix under test. */
export function hostBinaryCandidates(
  backend: Backend,
  { platform = process.platform, arch = process.arch, packageDir = resolve(import.meta.dir, "..") }: {
    platform?: string;
    arch?: string;
    packageDir?: string;
  } = {},
): Candidates {
  const key = hostPlatformKey(platform, arch);
  const binaryName = platform === "win32" ? `${BINARY_NAMES[backend]}.exe` : BINARY_NAMES[backend];
  const repoRoot = resolve(packageDir, "..", "..");
  const fresh = backend === "gtk"
    ? [resolve(repoRoot, "zig-out", "bin", "nd-hello")]
    : [resolve(repoRoot, "swift", ".build", "release", "NDShell"), resolve(repoRoot, "swift", ".build", "debug", "NDShell")];
  return { packageName: hostPackageName(backend, key), binaryName, repoRoot, fresh };
}

/**
 * The prebuilt binary from the installed platform package, or undefined when
 * the package is absent for this machine or its bin/ is empty. The existsSync
 * guard matters inside this repo: `bun install` symlinks the workspace
 * platform packages with no staged binary, so resolve can succeed while the
 * file itself is missing.
 */
export function prebuiltHostBinary(backend: Backend): string | undefined {
  const { packageName, binaryName } = hostBinaryCandidates(backend);
  if (!packageName) return undefined;
  try {
    const path = createRequire(import.meta.url).resolve(`${packageName}/bin/${binaryName}`);
    return existsSync(path) ? path : undefined;
  } catch {
    return undefined;
  }
}

/** A checkout of the NativeDesktop monorepo, where source builds are possible. */
function isSourceCheckout(repoRoot: string): boolean {
  return existsSync(resolve(repoRoot, "build.zig")) && existsSync(resolve(repoRoot, "swift", "Package.swift"));
}

/**
 * Absolute path to the host binary for the requested backend, building it on
 * first run when inside a source checkout. Resolution order per backend:
 *   1. prebuilt binary from the installed @nativedesktop/host-<os>-<arch> package
 *   2. (source checkout only) freshly built zig-out / swift .build artifacts
 *   3. (source checkout only) build the backend, then return the built artifact
 * Outside a checkout with no prebuilt, throws naming the missing platform
 * package (or the supported target list when none exists for this machine).
 */
export async function resolveHostBinary(
  opts: { backend?: Backend; platform?: string; arch?: string; packageDir?: string } = {},
): Promise<string> {
  const backend = resolveBackend(opts);
  const { packageName, binaryName, repoRoot, fresh } = hostBinaryCandidates(backend, opts);
  const prebuilt = prebuiltHostBinary(backend);
  const source = isSourceCheckout(repoRoot);

  if (prebuilt) {
    // In a source checkout, a newer zig-out/swift artifact wins over a stale
    // prebuilt; otherwise every dev/e2e run silently tests whatever was last
    // staged into the platform package, not the code just built (a real bite:
    // a Jul 16 prebuilt masked an entire wave of terminal fixes).
    if (source) {
      const built = fresh.find(existsSync);
      if (built && statSync(built).mtimeMs > statSync(prebuilt).mtimeMs) return built;
    }
    return prebuilt;
  }

  if (source) {
    const built = fresh.find(existsSync);
    if (built) return built;
    return buildBackend(backend, repoRoot, fresh);
  }

  const key = hostPlatformKey(opts.platform, opts.arch);
  // gtk-on-macOS is the one combination that will never get a package entry
  // (see PLATFORM_PACKAGES above), so the generic "unsupported target" hint
  // below reads as a dead end instead of the three real ways forward. State
  // them explicitly: this is by design, not missing packaging.
  if (!packageName && backend === "gtk" && key.startsWith("darwin-")) {
    throw new Error(
      `@nativedesktop/host: no gtk host binary for "${key}". The gtk backend ships no macOS prebuilt ` +
        `by design (it links Homebrew paths). Either build one from a NativeDesktop source checkout ` +
        `(${backendBuildHint(backend)}), or pass an explicit binary path instead of resolving one ` +
        `(e.g. launchApp({ hostBinary }) in @nativedesktop/test).`,
    );
  }
  const hint = packageName
    ? `Expected ${packageName}/bin/${binaryName} (an optionalDependency of @nativedesktop/host); ` +
      `reinstall without --no-optional, or build in a NativeDesktop checkout (${backendBuildHint(backend)}).`
    : `No prebuilt package exists for this target; supported targets are darwin-arm64 (appkit) and ` +
      `linux-x64 (gtk). Build in a NativeDesktop checkout (${backendBuildHint(backend)}).`;
  throw new Error(`@nativedesktop/host: no ${backend} host binary for "${key}". ${hint}`);
}

function backendBuildHint(backend: Backend): string {
  return backend === "gtk"
    ? "`zig build`"
    : "`zig build libnd -Dbackend=abi` then `cd swift && swift build -c release`";
}

/** Build the requested backend from source, logging one line, and return the artifact. */
async function buildBackend(backend: Backend, repoRoot: string, fresh: string[]): Promise<string> {
  process.stderr.write(`nd: building ${backend} host (first run)…\n`);
  if (backend === "gtk") {
    await run(["zig", "build"], repoRoot, process.env);
    const out = resolve(repoRoot, "zig-out", "bin", "nd-hello");
    if (!existsSync(out)) throw new Error(`@nativedesktop/host: gtk build produced no ${out}`);
    return out;
  }
  // appkit: build the GTK-free static core, repack the archive for Apple's ld,
  // then link the Swift shell. Same recipe as scripts/mac/build-appkit-host.sh.
  const env = appkitBuildEnv();
  await run(["zig", "build", "libnd", "-Dbackend=abi"], repoRoot, env);
  await repackLibnd(repoRoot, env);
  await run(["swift", "build", "-c", "release"], resolve(repoRoot, "swift"), env);
  const out = resolve(repoRoot, "swift", ".build", "release", "NDShell");
  if (!existsSync(out)) throw new Error(`@nativedesktop/host: appkit build produced no ${out}`);
  return out;
}

/** Zig's archiver emits members Apple's ld rejects ("not 8-byte aligned") and
 * extracts them 0-permission; repack with the system ar/libtool before linking. */
async function repackLibnd(repoRoot: string, env: Record<string, string | undefined>): Promise<void> {
  const lib = resolve(repoRoot, "zig-out", "lib", "libnd.a");
  const recipe = `workdir="$(mktemp -d)"; cd "$workdir" && ar x "${lib}" && chmod 644 *.o && libtool -static -o "${lib}" *.o; status=$?; rm -rf "$workdir"; exit $status`;
  await run(["bash", "-euo", "pipefail", "-c", recipe], repoRoot, env);
}

/** Env for the appkit build: scrub Nix/SDK overrides that break xcrun/swiftc so
 * the system Swift toolchain resolves its own SDK (mirrors quick-start's
 * `env -u SDKROOT -u DEVELOPER_DIR` and packages/nd's plugin build env). */
function appkitBuildEnv(): Record<string, string | undefined> {
  const env: Record<string, string | undefined> = { ...process.env };
  delete env.SDKROOT;
  delete env.DEVELOPER_DIR;
  delete env.DEVELOPER_SDK_DIR;
  delete env.NIX_CFLAGS_COMPILE;
  delete env.NIX_LDFLAGS;
  return env;
}

async function run(command: string[], cwd: string, env: Record<string, string | undefined>): Promise<void> {
  const proc = Bun.spawn(command, { cwd, env, stdin: "ignore", stdout: "inherit", stderr: "inherit" });
  const status = await proc.exited;
  if (status !== 0) throw new Error(`@nativedesktop/host: build step failed (${command.join(" ")})`);
}
