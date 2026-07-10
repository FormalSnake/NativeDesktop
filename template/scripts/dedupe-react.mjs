// Runs as a postinstall step. `@nativedesktop/react` is referenced via a `file:`
// path straight into a NativeDesktop framework checkout (the package isn't
// npm-published until M9 — see M8-D6 / template/README.md). Because that path
// keeps the package at its real location inside the framework's monorepo,
// Node/Bun module resolution for `@nativedesktop/react`'s own `import "react"`
// walks up from THAT location and finds the monorepo's hoisted `react`/
// `react-reconciler` — a different copy than the one this app's own
// `node_modules` installed, which breaks React's hooks dispatcher ("Invalid
// hook call") because two separate module instances exist.
//
// Fix: point this app's own node_modules/{react,react-reconciler} at the exact
// same resolved package directories the linked @nativedesktop/react sees, so
// there is only ever one copy of each loaded. This is a workaround for
// packages/react not yet declaring react/react-reconciler as peerDependencies
// (the standard fix for this class of bug); remove this script once it does.
import { existsSync, realpathSync, rmSync, symlinkSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";

const require = createRequire(import.meta.url);

const ndReactMain = require.resolve("@nativedesktop/react");
const ndReactDir = dirname(realpathSync(ndReactMain));

for (const dep of ["react", "react-reconciler"]) {
  const resolved = require.resolve(dep, { paths: [ndReactDir] });
  let dir = dirname(resolved);
  while (!existsSync(join(dir, "package.json"))) dir = dirname(dir);

  const target = join(process.cwd(), "node_modules", dep);
  if (existsSync(target) && realpathSync(target) === realpathSync(dir)) continue; // already aligned

  rmSync(target, { recursive: true, force: true });
  symlinkSync(dir, target, "dir");
  console.log(`dedupe-react: node_modules/${dep} -> ${dir}`);
}
