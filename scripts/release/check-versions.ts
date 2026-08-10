// Asserts every publishable package.json version equals the release version
// (the tag minus its "v" prefix). Run from the repo root:
//   bun scripts/release/check-versions.ts 0.1.0
import { resolve } from "node:path";

export const PUBLISHABLE = [
  "packages/host-darwin-arm64",
  "packages/host-linux-x64",
  "packages/host",
  "packages/react",
  "packages/native",
  "packages/data",
  "packages/panes",
  "packages/babel-plugin-nativedesktop",
  "packages/nd",
  // packages/test is deliberately unpublished: its client relative-imports
  // packages/mcp/src/socket.ts and react's generated rpc types, which only
  // resolve inside this checkout. Re-add once that import boundary is fixed.
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
if (failed) process.exit(1);
console.log(`all ${PUBLISHABLE.length} publishable packages are at ${expected}`);
