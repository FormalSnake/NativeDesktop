// @nativedesktop/host — resolves the host binary that draws the app for the
// current platform. Two backends exist: the GTK/Zig host (`nd-hello`, native on
// Linux, GTK-via-Quartz on macOS) and the AppKit/SwiftPM host (`nd-shell`,
// macOS-only). The default follows the platform — appkit on darwin, gtk on
// linux — and is overridable by an explicit `{ backend }` option or the
// ND_BACKEND env var. Binaries live under bin/<os>-<arch>/<name>, the
// Electron-style "prebuilt native binary as an npm dependency" model.
//
// How binaries get populated (there is no CI binary matrix yet):
//   - local dev (this repo): `zig build` produces zig-out/bin/nd-hello and the
//     appkit build produces swift/.build/release/NDShell; each is copied into
//     bin/<os>-<arch>/ (nd-hello / nd-shell) so a checkout runs with no download
//     step. bin/ is gitignored — the files are built locally or dropped by a
//     future per-platform @nativedesktop/host release, and shipped via the npm
//     `files` array.
//   - source-checkout fallback: when this package sits inside the NativeDesktop
//     repo, a missing prebuilt falls back to the freshly built artifacts, and if
//     those are missing too the requested backend is built on first run.
import { existsSync, statSync } from "node:fs";
import { resolve } from "node:path";

const OS_NAMES: Record<string, string> = { darwin: "darwin", linux: "linux", win32: "windows" };
const ARCH_NAMES: Record<string, string> = { arm64: "arm64", x64: "x64" };

export type Backend = "gtk" | "appkit";

/** Prebuilt binary basename per backend (matches the bin/<key>/ layout). */
const BINARY_NAMES: Record<Backend, string> = { gtk: "nd-hello", appkit: "nd-shell" };

/** `<os>-<arch>` key matching a `bin/` subdirectory, e.g. "darwin-arm64". */
export function hostPlatformKey(platform: string = process.platform, arch: string = process.arch): string {
  const os = OS_NAMES[platform];
  const cpu = ARCH_NAMES[arch];
  if (!os || !cpu) {
    throw new Error(`@nativedesktop/host: unsupported platform "${platform}-${arch}"`);
  }
  return `${os}-${cpu}`;
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
  /** Prebuilt binary shipped under bin/<key>/. */
  prebuilt: string;
  /** Repo root two levels up from the package, for the source-checkout fallback. */
  repoRoot: string;
  /** Freshly built artifacts, in preference order. */
  fresh: string[];
}

/** Pure path computation for a backend on a given platform — the resolution matrix under test. */
export function hostBinaryCandidates(
  backend: Backend,
  { platform = process.platform, arch = process.arch, packageDir = resolve(import.meta.dir, "..") }: {
    platform?: string;
    arch?: string;
    packageDir?: string;
  } = {},
): Candidates {
  const key = hostPlatformKey(platform, arch);
  const exe = platform === "win32" ? `${BINARY_NAMES[backend]}.exe` : BINARY_NAMES[backend];
  const prebuilt = resolve(packageDir, "bin", key, exe);
  const repoRoot = resolve(packageDir, "..", "..");
  const fresh = backend === "gtk"
    ? [resolve(repoRoot, "zig-out", "bin", "nd-hello")]
    : [resolve(repoRoot, "swift", ".build", "release", "NDShell"), resolve(repoRoot, "swift", ".build", "debug", "NDShell")];
  return { prebuilt, repoRoot, fresh };
}

/** A checkout of the NativeDesktop monorepo, where source builds are possible. */
function isSourceCheckout(repoRoot: string): boolean {
  return existsSync(resolve(repoRoot, "build.zig")) && existsSync(resolve(repoRoot, "swift", "Package.swift"));
}

/**
 * Absolute path to the host binary for the requested backend, building it on
 * first run when inside a source checkout. Resolution order per backend:
 *   1. prebuilt bin/<os>-<arch>/<name>
 *   2. (source checkout only) freshly built zig-out / swift .build artifacts
 *   3. (source checkout only) build the backend, then return the built artifact
 * Outside a checkout with no prebuilt, throws with the exact build command.
 */
export async function resolveHostBinary(opts: { backend?: Backend } = {}): Promise<string> {
  const backend = resolveBackend(opts);
  const { prebuilt, repoRoot, fresh } = hostBinaryCandidates(backend);
  const source = isSourceCheckout(repoRoot);

  if (existsSync(prebuilt)) {
    // In a source checkout, a newer zig-out/swift artifact wins over a stale
    // prebuilt — otherwise every dev/e2e run silently tests whatever was last
    // copied into bin/<key>/, not the code just built (a real bite: a Jul 16
    // prebuilt masked an entire wave of terminal fixes).
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

  throw new Error(
    `@nativedesktop/host: no ${backend} host binary for "${hostPlatformKey()}" at ${prebuilt}. ` +
      `Build it in a NativeDesktop checkout (${backendBuildHint(backend)}) and copy the result into ` +
      `${prebuilt}, or wait for a published per-platform @nativedesktop/host release ` +
      `(the CI binary matrix is not built yet).`,
  );
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
  // then link the Swift shell. Same recipe as scripts/mac/mac-run.sh / mac.yml.
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
