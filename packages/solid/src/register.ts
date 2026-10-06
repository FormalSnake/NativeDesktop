// Makes `bun <entry>.tsx` able to run a Solid app. It has to load before
// anything imports solid-js, so it runs as a preload: `nd dev`, `nd package`
// and @nativedesktop/test's launchApp put `--preload=@nativedesktop/solid/register`
// in BUN_OPTIONS for an app that depends on @nativedesktop/solid
// (@nativedesktop/host's preload.ts), and this package's bunfig.toml preloads
// it for `bun test`.
//
// Two jobs, both runtime Bun plugins:
// - solid-js's package exports send the "node" condition, which Bun's runtime
//   always sets, to its SSR build: signals never re-run there. Runtime plugins
//   cannot intercept a bare specifier Bun resolves itself (onResolve never
//   fires for it), so the SSR file is replaced at load time with a re-export
//   of the client build instead.
// - .jsx/.tsx go through babel-preset-solid's universal transform, whose
//   compiled output imports its helpers from @nativedesktop/solid. A file a
//   plugin returns contents for drops out of `bun --hot`'s watch set, so
//   component edits need a restart: there is no Solid HMR yet.
/// <reference path="./babel-presets.d.ts" />
import { transformAsync } from "@babel/core";
import presetTypescript from "@babel/preset-typescript";
import presetSolid from "babel-preset-solid";
import { dirname, join } from "node:path";

const solidDir = dirname(require.resolve("solid-js/package.json"));
const clientBuild = join(solidDir, "dist", "solid.js");

Bun.plugin({
  name: "nativedesktop-solid",
  setup(build) {
    build.onLoad({ filter: /[\\/]solid-js[\\/]dist[\\/]server(\.dev)?\.js$/ }, () => ({
      contents: `export * from ${JSON.stringify(clientBuild)};`,
      loader: "js",
    }));
    build.onLoad({ filter: /\.[jt]sx$/ }, async (args) => {
      const source = await Bun.file(args.path).text();
      const out = await transformAsync(source, {
        filename: args.path,
        babelrc: false,
        configFile: false,
        sourceMaps: "inline",
        presets: [
          [presetTypescript, { isTSX: true, allExtensions: true }],
          [presetSolid, { generate: "universal", moduleName: "@nativedesktop/solid" }],
        ],
      });
      if (!out?.code) throw new Error(`babel-preset-solid produced no output for ${args.path}`);
      return { contents: out.code, loader: "js" };
    });
  },
});
