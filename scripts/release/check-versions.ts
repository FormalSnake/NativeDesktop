// Asserts every publishable package.json version equals the release version
// (the tag minus its "v" prefix), and that bun.lock records the same version
// for each of them. Run from the repo root:
//   bun scripts/release/check-versions.ts 0.1.0
//
// The lockfile half is not redundant: `bun pm pack` rewrites workspace:* deps
// from the lock, not from the sibling package.json, and `bun install
// --frozen-lockfile` does not fail on a workspace version the lock predates.
// v0.1.1 shipped that way -- every package at 0.1.1 depending on 0.1.0
// siblings, so consumers installed 0.1.1 TypeScript against 0.1.0 host
// binaries. A version bump must be committed together with its `bun install`.
import { resolve } from "node:path";

export const PUBLISHABLE = [
  "packages/host-darwin-arm64",
  "packages/host-linux-x64",
  "packages/host",
  "packages/react",
  "packages/rpc",
  "packages/native",
  "packages/data",
  "packages/panes",
  "packages/ui",
  "packages/babel-plugin-nativedesktop",
  "packages/nd",
  "packages/test",
];

const expected = process.argv[2];
if (!expected) {
  console.error("usage: check-versions.ts <version>");
  process.exit(2);
}

const root = resolve(import.meta.dir, "..", "..");
let failed = false;
for (const dir of PUBLISHABLE) {
  const pkg = await Bun.file(resolve(root, dir, "package.json")).json();
  if (pkg.version !== expected) {
    console.error(`${dir}: version ${pkg.version} != ${expected}`);
    failed = true;
  }
}

// bun.lock is JSONC (trailing commas, no comments); nothing in it can produce
// a "," before a closing brace inside a string, so dropping those is enough.
const lock = JSON.parse((await Bun.file(resolve(root, "bun.lock")).text()).replace(/,(\s*[}\]])/g, "$1"));
for (const dir of PUBLISHABLE) {
  const recorded = lock.workspaces?.[dir]?.version;
  if (recorded !== expected) {
    console.error(`bun.lock: ${dir} recorded at ${recorded} != ${expected} — run \`bun install\` and commit the lockfile`);
    failed = true;
  }
}

if (failed) process.exit(1);
console.log(`all ${PUBLISHABLE.length} publishable packages, and bun.lock, are at ${expected}`);
