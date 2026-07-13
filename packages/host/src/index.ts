// @nativedesktop/host — resolves the prebuilt `nd-hello` binary for the
// current platform, the Electron-style "prebuilt native binary as an npm
// dependency" model. Binaries live under bin/<os>-<arch>/nd-hello.
//
// How binaries get populated (no CI matrix yet — that's future scope):
//   - local dev (this repo): `zig build` at the framework repo root produces
//     zig-out/bin/nd-hello for the host's own platform; it is copied into
//     bin/<os>-<arch>/nd-hello here so `nd dev` works against a checkout with
//     no separate download step (see packages/host/bin/darwin-arm64/).
//   - external consumers (future): a per-platform-published @nativedesktop/host
//     release (or postinstall download step) drops the matching binary into
//     the same bin/<os>-<arch>/ layout — resolveHostBinary() doesn't care how
//     the file got there, only that it exists at the conventional path.
import { existsSync } from "node:fs";
import { resolve } from "node:path";

const OS_NAMES: Record<string, string> = { darwin: "darwin", linux: "linux", win32: "windows" };
const ARCH_NAMES: Record<string, string> = { arm64: "arm64", x64: "x64" };

/** `<os>-<arch>` key matching a `bin/` subdirectory, e.g. "darwin-arm64". */
export function hostPlatformKey(platform: string = process.platform, arch: string = process.arch): string {
  const os = OS_NAMES[platform];
  const cpu = ARCH_NAMES[arch];
  if (!os || !cpu) {
    throw new Error(`@nativedesktop/host: unsupported platform "${platform}-${arch}"`);
  }
  return `${os}-${cpu}`;
}

/** Absolute path to the prebuilt `nd-hello` binary for the running platform. */
export function resolveHostBinary(): string {
  const key = hostPlatformKey();
  const exe = key.startsWith("windows-") ? "nd-hello.exe" : "nd-hello";
  const binPath = resolve(import.meta.dir, "..", "bin", key, exe);
  if (!existsSync(binPath)) {
    throw new Error(
      `@nativedesktop/host: no prebuilt nd-hello for "${key}" at ${binPath}. ` +
        `Run \`zig build\` at the NativeDesktop repo root and copy zig-out/bin/nd-hello ` +
        `into packages/host/bin/${key}/nd-hello, or wait for a published per-platform ` +
        `@nativedesktop/host release (the CI binary matrix is not built yet).`,
    );
  }
  return binPath;
}
