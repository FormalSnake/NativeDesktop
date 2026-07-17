#!/usr/bin/env bun
// nd — the NativeDesktop CLI. Wraps the invocations documented in
// docs/agents/README.md / template/AGENTS.md so there is one real command
// instead of a hand-typed env-var incantation:
//   `nd dev [entry]`  ==  ND_DEV=1 ND_SCRIPT=<entry> <host-binary-from-@nativedesktop/host>
//   `nd build`        ==  bun run compile   (babel + react-compiler pre-pass, see template/README.md)
import { type Backend, resolveHostBinary } from "@nativedesktop/host";
import { buildNativePlugins, loadConfig } from "./config.ts";

const DEFAULT_ENTRY = "src/main.tsx";

async function nativeEnv(): Promise<Record<string, string>> {
  const paths = await buildNativePlugins(await loadConfig());
  return paths.length ? { ND_PLUGINS: "1", ND_PLUGIN_PATHS: paths.join(":") } : {};
}

async function runDev(entry: string, backend?: Backend): Promise<number> {
  const hostBinary = await resolveHostBinary({ backend });
  const proc = Bun.spawn([hostBinary], {
    cwd: process.cwd(),
    env: { ...process.env, ...(await nativeEnv()), ND_DEV: "1", ND_SCRIPT: entry },
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
    ].join("\n"),
  );
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
    default:
      if (cmd) console.error(`nd: unknown command "${cmd}"\n`);
      usage();
      process.exit(1);
  }
}

if (import.meta.main) await main();
