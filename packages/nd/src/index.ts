#!/usr/bin/env bun
// nd — the NativeDesktop CLI. Wraps the invocations documented in
// docs/agents/README.md / template/AGENTS.md so there is one real command
// instead of a hand-typed env-var incantation:
//   `nd dev [entry]`  ==  ND_DEV=1 ND_SCRIPT=<entry> <host-binary-from-@nativedesktop/host>
//   `nd build`        ==  bun run compile   (babel + react-compiler pre-pass, see template/README.md)
import { resolveHostBinary } from "@nativedesktop/host";
import { buildNativePlugins, loadConfig } from "./config.ts";

const DEFAULT_ENTRY = "src/main.tsx";

async function nativeEnv(): Promise<Record<string, string>> {
  const paths = await buildNativePlugins(await loadConfig());
  return paths.length ? { ND_PLUGINS: "1", ND_PLUGIN_PATHS: paths.join(process.platform === "win32" ? ";" : ":") } : {};
}

async function runDev(entry: string): Promise<number> {
  const hostBinary = resolveHostBinary();
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
  await nativeEnv();
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
      "  nd dev [entry]   run the app in dev mode (ND_DEV=1, hot reload + crash overlay)",
      `                   entry defaults to "${DEFAULT_ENTRY}"`,
      "  nd build         compile the app for production (bun run compile)",
    ].join("\n"),
  );
}

async function main(): Promise<void> {
  const [cmd, ...rest] = process.argv.slice(2);
  switch (cmd) {
    case "dev":
      process.exit(await runDev(rest[0] ?? DEFAULT_ENTRY));
      break;
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
