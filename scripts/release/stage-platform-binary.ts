// Stages a built host binary into its platform package before publish:
//   bun scripts/release/stage-platform-binary.ts <darwin-arm64|linux-x64> <artifact> [<cef-helper>]
// Copies into packages/host-<target>/bin/<exe>, chmods 755, prints the size,
// and on linux-x64 prints ldd output so the release log records the sonames.
import { chmodSync, copyFileSync, existsSync, mkdirSync, statSync } from "node:fs";
import { dirname, resolve } from "node:path";

const EXES: Record<string, string> = { "darwin-arm64": "nd-shell", "linux-x64": "nd-hello" };

// darwin-arm64 also stages nd-cef-helper beside the host: `nd package mac`
// looks for it there (packages/nd/src/package/cef.ts). Linux has none, CEF
// re-execs nd-hello itself.
const [target, artifact, helper] = process.argv.slice(2);
const exe = target ? EXES[target] : undefined;
if (!exe || !artifact) {
  console.error("usage: stage-platform-binary.ts <darwin-arm64|linux-x64> <artifact> [<cef-helper>]");
  process.exit(2);
}
if (!existsSync(artifact)) {
  console.error(`stage-platform-binary: no such artifact ${artifact}`);
  process.exit(1);
}

if (target === "darwin-arm64" && !helper) {
  console.error("stage-platform-binary: darwin-arm64 needs the nd-cef-helper artifact as a third argument");
  process.exit(2);
}
if (helper && !existsSync(helper)) {
  console.error(`stage-platform-binary: no such artifact ${helper}`);
  process.exit(1);
}

const root = resolve(import.meta.dir, "..", "..");
const dest = resolve(root, `packages/host-${target}`, "bin", exe);
mkdirSync(dirname(dest), { recursive: true });
copyFileSync(artifact, dest);
chmodSync(dest, 0o755);
console.log(`${dest}: ${statSync(dest).size} bytes`);

if (helper) {
  const helperDest = resolve(dirname(dest), "nd-cef-helper");
  copyFileSync(helper, helperDest);
  chmodSync(helperDest, 0o755);
  console.log(`${helperDest}: ${statSync(helperDest).size} bytes`);
}

if (target === "linux-x64" && process.platform === "linux") {
  const ldd = Bun.spawnSync(["ldd", dest]);
  console.log(ldd.stdout.toString());
  if (ldd.exitCode !== 0) {
    console.error(`ldd failed: ${ldd.stderr.toString()}`);
    process.exit(1);
  }
}
