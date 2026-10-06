#!/usr/bin/env bun
// `nd-solid-build [entry] [--outdir dir]`: compiles a Solid app ahead of time
// for `nd build` / `nd package`, which run it as the app's `compile` script.
// The app's own modules bundle into one <outdir>/<entry name>.js with the
// universal transform already applied; packages stay external imports, so
// solid-js still resolves through the register preload at launch and lands
// on its client build there.
import { basename } from "node:path";
import { transformSolid } from "./transform.ts";

export interface BuildOptions {
  entry: string;
  outdir: string;
}

export async function buildApp({ entry, outdir }: BuildOptions): Promise<string> {
  const result = await Bun.build({
    entrypoints: [entry],
    outdir,
    target: "bun",
    packages: "external",
    plugins: [
      {
        name: "nativedesktop-solid",
        setup(build) {
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
