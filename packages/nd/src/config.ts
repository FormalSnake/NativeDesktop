import { existsSync, statSync } from "node:fs";
import { resolve } from "node:path";
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

export interface NativeDesktopConfig {
  native?: { plugins?: NativePluginConfig[] };
}

export function defineConfig(config: NativeDesktopConfig): NativeDesktopConfig { return config; }

export async function loadConfig(cwd = process.cwd()): Promise<NativeDesktopConfig> {
  const path = resolve(cwd, "nativedesktop.config.ts");
  if (!existsSync(path)) return {};
  const mod = await import(`${pathToFileURL(path).href}?t=${statSync(path).mtimeMs}`);
  return (mod.default ?? mod.config ?? {}) as NativeDesktopConfig;
}

export async function buildNativePlugins(config: NativeDesktopConfig, cwd = process.cwd()): Promise<string[]> {
  const platform = process.platform;
  if (platform !== "darwin" && platform !== "linux") return [];
  const outputs: string[] = [];
  for (const plugin of config.native?.plugins ?? []) {
    const output = resolve(cwd, plugin[platform] ?? plugin.path ?? "");
    if (!output || output === cwd) throw new Error("nd: native plugin requires path, darwin, or linux output");
    const configured = plugin.build;
    const build = configured && "command" in configured
      ? configured as NativeBuildCommand
      : (configured as Partial<Record<"darwin" | "linux", NativeBuildCommand>> | undefined)?.[platform];
    if (build && needsBuild(output, ["nativedesktop.config.ts", ...(build.inputs ?? [])], cwd)) {
      const proc = Bun.spawn(build.command, { cwd, env: process.env, stdin: "inherit", stdout: "inherit", stderr: "inherit" });
      const status = await proc.exited;
      if (status !== 0) throw new Error(`nd: native build failed (${build.command.join(" ")})`);
    }
    if (!existsSync(output)) throw new Error(`nd: native plugin output not found: ${output}`);
    outputs.push(output);
  }
  return outputs;
}

function needsBuild(output: string, inputs: string[], cwd: string): boolean {
  if (!existsSync(output)) return true;
  const outputTime = statSync(output).mtimeMs;
  return inputs.some((input) => newestMtime(resolve(cwd, input)) > outputTime);
}

function newestMtime(path: string): number {
  if (!existsSync(path)) return 0;
  const stat = statSync(path);
  if (!stat.isDirectory()) return stat.mtimeMs;
  const glob = new Bun.Glob("**/*");
  let newest = stat.mtimeMs;
  for (const relative of glob.scanSync({ cwd: path, onlyFiles: true })) newest = Math.max(newest, statSync(resolve(path, relative)).mtimeMs);
  return newest;
}
