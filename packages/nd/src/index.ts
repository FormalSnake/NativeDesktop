#!/usr/bin/env bun
// nd — the NativeDesktop CLI. Wraps the invocations documented in
// docs/agents/README.md / template/AGENTS.md so there is one real command
// instead of a hand-typed env-var incantation:
//   `nd dev [entry]`  ==  ND_DEV=1 ND_SCRIPT=<entry> <host-binary-from-@nativedesktop/host>
//   `nd build`        ==  bun run compile   (babel + react-compiler pre-pass, see template/README.md)
//   `nd package`      ==  assemble + sign the platform bundle (packages/nd/src/package/)
//   `nd doctor`       ==  packaging/toolchain readiness checks
import { type Backend, resolveHostBinary } from "@nativedesktop/host";
import {
  buildNativePlugins,
  engineTargetFor,
  loadConfig,
  type NativeDesktopConfig,
  resolveCefSchemes,
  resolveWebViewEngine,
} from "./config.ts";
import { packageApp, type PackageOptions } from "./package/index.ts";

const DEFAULT_ENTRY = "src/main.tsx";

async function nativeEnv(config: NativeDesktopConfig): Promise<Record<string, string>> {
  const paths = await buildNativePlugins(config);
  return paths.length ? { ND_PLUGINS: "1", ND_PLUGIN_PATHS: paths.join(":") } : {};
}

/** The host reads the engine and its launch-declared schemes off its
 * environment, so both config decisions have to be made here and handed down.
 * Windows has no engine target yet. */
function engineEnv(config: NativeDesktopConfig): Record<string, string> {
  const target = engineTargetFor();
  if (!target) return {};
  const schemes = resolveCefSchemes(config);
  return {
    ND_WEBVIEW_ENGINE: resolveWebViewEngine(config, target),
    ...(schemes.length ? { ND_CEF_SCHEMES: schemes.join(",") } : {}),
  };
}

async function runDev(entry: string, backend?: Backend): Promise<number> {
  const hostBinary = await resolveHostBinary({ backend });
  const config = await loadConfig();
  const proc = Bun.spawn([hostBinary], {
    cwd: process.cwd(),
    env: { ...process.env, ...(await nativeEnv(config)), ...engineEnv(config), ND_DEV: "1", ND_SCRIPT: entry },
    stdin: "inherit",
    stdout: "inherit",
    stderr: "inherit",
  });
  return proc.exited;
}

async function runBuild(): Promise<number> {
  // Production plugin wiring is the app's packaging responsibility; we only
  // make sure the native artifacts are built and fresh.
  await buildNativePlugins(await loadConfig());
  const proc = Bun.spawn(["bun", "run", "compile"], {
    cwd: process.cwd(),
    env: process.env,
    stdin: "inherit",
    stdout: "inherit",
    stderr: "inherit",
  });
  return proc.exited;
}

function usage(): void {
  console.error(
    [
      "Usage:",
      "  nd dev [entry] [--backend gtk|appkit]",
      "                   run the app in dev mode (ND_DEV=1, hot reload + crash overlay)",
      `                   entry defaults to "${DEFAULT_ENTRY}"`,
      "                   backend defaults to appkit on macOS, gtk elsewhere (also honors ND_BACKEND);",
      "                   --backend gtk cross-checks the GTK host, --backend appkit forces the macOS shell",
      "  nd build         compile the app for production (bun run compile)",
      "  nd package [mac|linux] [--out <dir>] [--entry <file>] [--version <v>] [--cwd <dir>]",
      "             [--no-compile] [--sign <identity>|--no-sign] [--notarize|--no-notarize]",
      "             [--format appimage|appdir]",
      "                   assemble + sign the platform bundle (platform defaults to the host;",
      "                   Windows lands with M7)",
      "  nd doctor [--json]",
      "                   check packaging/toolchain readiness for the current directory",
    ].join("\n"),
  );
}

/** Parse `nd package` args. Exits 2 on an unsupported platform (Windows unchanged). */
export function parsePackageArgs(args: string[], platform: string = process.platform): PackageOptions {
  let target: PackageOptions["platform"] | undefined;
  const opts: Omit<PackageOptions, "platform"> = {};
  const takeValue = (flag: string, value: string | undefined): string => {
    if (value === undefined) {
      console.error(`nd: ${flag} expects a value\n`);
      usage();
      process.exit(1);
    }
    return value;
  };
  for (let i = 0; i < args.length; i++) {
    const arg = args[i]!;
    if (arg === "--out") opts.outDir = takeValue(arg, args[++i]);
    else if (arg === "--entry") opts.entry = takeValue(arg, args[++i]);
    else if (arg === "--version") opts.version = takeValue(arg, args[++i]);
    else if (arg === "--cwd") opts.cwd = takeValue(arg, args[++i]);
    else if (arg === "--no-compile") opts.compile = false;
    else if (arg === "--sign") opts.signIdentity = takeValue(arg, args[++i]);
    else if (arg === "--no-sign") opts.signIdentity = null;
    else if (arg === "--notarize") opts.notarize = true;
    else if (arg === "--no-notarize") opts.notarize = false;
    else if (arg === "--format") {
      const format = takeValue(arg, args[++i]);
      if (format !== "appimage" && format !== "appdir") {
        console.error(`nd: --format expects "appimage" or "appdir" (got "${format}")\n`);
        usage();
        process.exit(1);
      }
      opts.format = format;
    } else if (!arg.startsWith("-") && target === undefined) {
      if (arg !== "mac" && arg !== "linux") {
        console.error(`nd package: unsupported platform "${arg}" (Windows lands with M7)\n`);
        usage();
        process.exit(2);
      }
      target = arg;
    } else {
      console.error(`nd: unexpected argument "${arg}"\n`);
      usage();
      process.exit(1);
    }
  }
  if (target === undefined) {
    if (platform === "darwin") target = "mac";
    else if (platform === "linux") target = "linux";
    else {
      console.error(`nd package: unsupported host platform "${platform}" (Windows lands with M7)\n`);
      usage();
      process.exit(2);
    }
  }
  return { platform: target, ...opts };
}

/** Parse `[entry] [--backend gtk|appkit]` in any order. Returns undefined backend to defer to ND_BACKEND / platform default. */
function parseDevArgs(args: string[]): { entry: string; backend?: Backend } {
  let entry: string | undefined;
  let backend: Backend | undefined;
  for (let i = 0; i < args.length; i++) {
    const arg = args[i]!;
    if (arg === "--backend") backend = expectBackend(args[++i]);
    else if (arg.startsWith("--backend=")) backend = expectBackend(arg.slice("--backend=".length));
    else if (!arg.startsWith("-") && entry === undefined) entry = arg;
    else {
      console.error(`nd: unexpected argument "${arg}"\n`);
      usage();
      process.exit(1);
    }
  }
  return { entry: entry ?? DEFAULT_ENTRY, backend };
}

function expectBackend(value: string | undefined): Backend {
  if (value !== "gtk" && value !== "appkit") {
    console.error(`nd: --backend expects "gtk" or "appkit" (got "${value ?? ""}")\n`);
    usage();
    process.exit(1);
  }
  return value;
}

async function main(): Promise<void> {
  const [cmd, ...rest] = process.argv.slice(2);
  switch (cmd) {
    case "dev": {
      const { entry, backend } = parseDevArgs(rest);
      process.exit(await runDev(entry, backend));
      break;
    }
    case "build":
      process.exit(await runBuild());
      break;
    case "package":
      await packageApp(parsePackageArgs(rest));
      process.exit(0);
      break;
    case "doctor": {
      const json = rest.includes("--json");
      const extra = rest.filter((arg) => arg !== "--json");
      if (extra.length) {
        console.error(`nd: unexpected argument "${extra[0]}"\n`);
        usage();
        process.exit(1);
      }
      const { runDoctor } = await import("./package/doctor.ts");
      process.exit(await runDoctor(process.cwd(), json));
      break;
    }
    default:
      if (cmd) console.error(`nd: unknown command "${cmd}"\n`);
      usage();
      process.exit(1);
  }
}

if (import.meta.main) await main();
