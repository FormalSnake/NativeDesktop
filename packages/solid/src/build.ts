#!/usr/bin/env bun
// `nd-solid-build [entry] [--outdir dir]`: compiles a Solid app ahead of time
// for `nd build` / `nd package`, which run it as the app's `compile` script.
// The app's own modules bundle into one <outdir>/<entry name>.js with the
// universal transform already applied, together with the renderer and its
// reactive core (`framework` below), built from solid-js's client export. Bun
// loads each ES module file separately at launch, and @solidjs/signals alone
// is some 30 of them. Every importer of solid-js has to be in the bundle too:
// one left external would load a second reactive graph the tree knows nothing
// about. Other packages stay external imports.
import { readFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { transformSolid } from "./transform.ts";

// The framework's Solid entry points by name. @nativedesktop/data and /rpc
// keep their main entries external: they import no solid-js, and data's
// sqlite worker is loaded from its own file.
const framework = /^(solid-js|@solidjs\/|@nativedesktop\/(solid|core|panes)$|@nativedesktop\/[^/]+\/solid$)/;

/// Whether `specifier`'s package imports solid-js itself, judged by its
/// package.json: a third-party Solid library is bundled for the same reason.
function importsSolid(specifier: string, from: string, seen: Map<string, boolean>): boolean {
  const name = specifier.startsWith("@") ? specifier.split("/").slice(0, 2).join("/") : specifier.split("/")[0]!;
  const known = seen.get(name);
  if (known !== undefined) return known;
  let uses = false;
  try {
    const pkg = JSON.parse(readFileSync(require.resolve(`${name}/package.json`, { paths: [from] }), "utf8"));
    uses = [pkg.dependencies, pkg.peerDependencies].some((d) => d && ("solid-js" in d || "@nativedesktop/solid" in d));
  } catch {}
  seen.set(name, uses);
  return uses;
}

/// solid-js and @solidjs/* resolved from the app, whoever imports them, into
/// their client production export. A linked framework checkout resolves them
/// from its own node_modules otherwise, and the bundle gets two reactive cores.
function reactiveCore(specifier: string, appDir: string): string | undefined {
  const parts = specifier.split("/");
  const name = specifier.startsWith("@") ? parts.slice(0, 2).join("/") : parts[0]!;
  const sub = "." + specifier.slice(name.length);
  let pkgFile: string;
  try {
    pkgFile = require.resolve(`${name}/package.json`, { paths: [appDir] });
  } catch {
    return undefined;
  }
  const exports = JSON.parse(readFileSync(pkgFile, "utf8")).exports;
  const pick = (e: unknown): string | undefined => {
    if (typeof e === "string") return e;
    if (!e || typeof e !== "object") return undefined;
    for (const [key, value] of Object.entries(e)) {
      if (key === "browser" || key === "import" || key === "default") return pick(value);
    }
    return undefined;
  };
  const target = pick(typeof exports === "object" && exports && sub in exports ? exports[sub] : sub === "." ? exports : undefined);
  return target ? join(dirname(pkgFile), target) : undefined;
}

export interface BuildOptions {
  entry: string;
  outdir: string;
}

export async function buildApp({ entry, outdir }: BuildOptions): Promise<string> {
  const result = await Bun.build({
    entrypoints: [entry],
    outdir,
    target: "bun",
    // solid-js's "node" export is its SSR build, where signals never re-run.
    conditions: ["browser"],
    // Bun.build resolves the "development" export unless NODE_ENV says
    // otherwise, and this is the production build.
    define: { "process.env.NODE_ENV": '"production"' },
    plugins: [
      {
        name: "nativedesktop-solid",
        setup(build) {
          const seen = new Map<string, boolean>();
          const appDir = dirname(resolve(entry));
          build.onResolve({ filter: /^(solid-js|@solidjs\/)/ }, (args) => {
            const path = reactiveCore(args.path, appDir);
            return path ? { path } : undefined;
          });
          // Bare specifiers only, and not the entry point, which arrives here too.
          build.onResolve({ filter: /^[^./]/ }, (args) => {
            if (args.kind.startsWith("entry-point") || /^(node|bun):/.test(args.path)) return undefined;
            if (framework.test(args.path)) return undefined;
            if (!args.path.startsWith("@nativedesktop/") && importsSolid(args.path, dirname(args.importer), seen)) return undefined;
            return { path: args.path, external: true };
          });
          build.onLoad({ filter: /\.[jt]sx$/ }, async (args) => {
            const source = await Bun.file(args.path).text();
            return { contents: await transformSolid(source, args.path), loader: "js" };
          });
        },
      },
    ],
  });
  if (!result.success) throw new AggregateError(result.logs, `nd-solid-build: building ${entry} failed`);
  const out = result.outputs.find((o) => o.kind === "entry-point");
  if (!out) throw new Error(`nd-solid-build: no output for ${entry}`);
  return out.path;
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  let entry = "src/main.tsx";
  let outdir = "dist";
  for (let i = 0; i < args.length; i++) {
    const arg = args[i]!;
    if (arg === "--outdir") outdir = args[++i] ?? outdir;
    else entry = arg;
  }
  const path = await buildApp({ entry, outdir });
  console.log(`nd-solid-build: ${entry} -> ${outdir}/${basename(path)}`);
}
