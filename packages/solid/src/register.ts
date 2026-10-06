// Makes `bun <entry>.tsx` able to run a Solid app. It has to load before
// anything imports solid-js, so it runs as a preload: `nd dev`, `nd package`
// and @nativedesktop/test's launchApp put `--preload=@nativedesktop/solid/register`
// in BUN_OPTIONS for an app that depends on @nativedesktop/solid
// (@nativedesktop/host's preload.ts), and this package's bunfig.toml preloads
// it for `bun test`.
//
// Runtime Bun plugins, all through one onLoad (which one of several matching
// onLoad hooks Bun picks is not specified):
// - solid-js's package exports send the "node" condition, which Bun's runtime
//   always sets, to its SSR build: signals never re-run there. Runtime plugins
//   cannot intercept a bare specifier Bun resolves itself (onResolve never
//   fires for it), so the SSR file is replaced at load time with a re-export
//   of the client build instead (the dev build under `nd dev`, which the
//   refresh runtime requires).
// - A Solid app's .jsx/.tsx go through babel-preset-solid's universal
//   transform, whose compiled output imports its helpers from
//   @nativedesktop/solid.
// - Under `nd dev` (ND_DEV=1, `bun --hot`), see the hot reload notes below.
/// <reference path="./babel-presets.d.ts" />
import { transformAsync, types as t, type PluginObj, type TransformOptions } from "@babel/core";
import presetTypescript from "@babel/preset-typescript";
import presetSolid from "babel-preset-solid";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, sep } from "node:path";

const dev = process.env.ND_DEV === "1";
// A native addon, needed for the dev refresh pass only; a packaged app never loads it.
const refreshCompiler = dev ? await import("@solidjs/compiler") : undefined;
const solidDir = dirname(require.resolve("solid-js/package.json"));
const clientBuild = join(solidDir, "dist", dev ? "solid.dev.js" : "solid.js");
const serverBuild = /[\\/]solid-js[\\/]dist[\\/]server(\.dev)?\.js$/;
const jsx = /\.[jt]sx$/;

// Hot reload. `bun --hot` re-evaluates every module on an edit, node_modules
// included, and keeps only globalThis. Two consequences:
// - Each component module compiles with Solid's refresh transform
//   (@solidjs/compiler's transformRefresh, the pass @solidjs/vite-plugin
//   runs) against the solid-js/refresh runtime, in its Vite mode. Bun
//   reserves `import.meta.hot` for its own dev server and reads it as
//   undefined here, so the transform's references are rewritten to a
//   per-module object from `__nd_solid_hot`, which plays Vite's part: a
//   module's previous accept callbacks run once its new copy has evaluated,
//   and those patch the live component proxies to the new code.
// - A fresh solid-js would own a second reactive graph that the live tree
//   knows nothing about, and a fresh @nativedesktop/solid would lose the
//   renderer's retained tree. So the reactive core, the universal renderer
//   and this package are pinned: the first evaluation of each file records
//   its namespace on globalThis, and every later load of the file is a facade
//   re-exporting that namespace. Pinning also keeps the identities the
//   refresh runtime compares (a component's `dependencies`) stable, so an
//   edit remounts only the components whose code changed.
const signalsDir = dirname(require.resolve("@solidjs/signals/package.json", { paths: [solidDir] }));
// Bun's conditions never include "development", so the dev builds the
// refresh runtime needs (its own, and solid-js's reactive core under the dev
// solid-js) are swapped in by path the same way.
const devRedirects = new Map<string, string>(
  dev
    ? [
        [join(signalsDir, "dist", "prod", "index.js"), join(signalsDir, "dist", "dev.js")],
        [join(solidDir, "dist", "refresh.js"), join(solidDir, "dist", "refresh.dev.js")],
      ]
    : [],
);
const pinnedDirs = dev
  ? [
      join(solidDir, "dist"),
      signalsDir,
      dirname(require.resolve("@solidjs/universal/package.json", { paths: [import.meta.dir] })),
      import.meta.dir,
    ].map((d) => d + sep)
  : [];
const isPinned = (path: string) =>
  path !== import.meta.path && /\.[cm]?[jt]s$/.test(path) && pinnedDirs.some((d) => path.startsWith(d));

declare global {
  // eslint-disable-next-line no-var
  var __nd_solid_pins: Map<string, Record<string, unknown>> | undefined;
}

interface HotContext {
  data: Record<string, unknown>;
  accept(cb?: (mod: unknown) => void): void;
  dispose(cb: (data: Record<string, unknown>) => void): void;
  decline(): void;
  invalidate(): void;
}

if (dev) {
  globalThis.__nd_solid_pins ??= new Map();
  const records = new Map<string, { data: Record<string, unknown>; accept: ((mod: unknown) => void)[] }>();
  globalThis.__nd_solid_hot = (id: string): HotContext => {
    let rec = records.get(id);
    const previous = rec?.accept ?? [];
    if (!rec) records.set(id, (rec = { data: {}, accept: [] }));
    rec.accept = [];
    const own = rec;
    // The new copy of the module is still evaluating; the previous copy's
    // callbacks expect to see it finished, which a macrotask guarantees.
    if (previous.length) setTimeout(() => previous.forEach((cb) => cb({})), 0);
    return {
      data: own.data,
      accept: (cb) => {
        if (cb) own.accept.push(cb);
      },
      dispose: () => {},
      decline: () => {},
      // A change the runtime cannot patch (a component removed): remount the
      // whole tree from the newest render() call instead.
      invalidate: () => globalThis.__nd_solid_remount?.(),
    };
  };
}

// A Solid .tsx is one whose nearest package.json is this package or depends
// on it; any other .tsx (React code in the same process, as under a
// repo-wide `bun test`) keeps Bun's own transform.
const solidPackages = new Map<string, boolean>();
function isSolidSource(path: string): boolean {
  for (let dir = dirname(path); ; dir = dirname(dir)) {
    const known = solidPackages.get(dir);
    if (known !== undefined) return known;
    const manifest = join(dir, "package.json");
    if (existsSync(manifest)) {
      const pkg = JSON.parse(readFileSync(manifest, "utf8")) as {
        name?: string;
        dependencies?: Record<string, string>;
        devDependencies?: Record<string, string>;
      };
      const solid =
        pkg.name === "@nativedesktop/solid" ||
        pkg.dependencies?.["@nativedesktop/solid"] !== undefined ||
        pkg.devDependencies?.["@nativedesktop/solid"] !== undefined;
      solidPackages.set(dir, solid);
      return solid;
    }
    if (dirname(dir) === dir) return false;
  }
}

function facade(path: string, ns: Record<string, unknown>): string {
  const key = JSON.stringify(path);
  let out = `const __nd_ns = globalThis.__nd_solid_pins.get(${key});\n`;
  for (const name of Object.keys(ns)) {
    if (name === "default") out += "export default __nd_ns.default;\n";
    else if (/^[A-Za-z_$][\w$]*$/.test(name)) out += `export const ${name} = __nd_ns.${name};\n`;
  }
  return out;
}

function selfPin(path: string, contents: string): string {
  const key = JSON.stringify(path);
  return `${contents}\nimport * as __nd_self from ${key};\nglobalThis.__nd_solid_pins.set(${key}, __nd_self);\n`;
}

/** Points the refresh transform's `import.meta.hot` at this module's `__nd_solid_hot` context. */
function hotContext(id: string): PluginObj {
  return {
    visitor: {
      Program(program) {
        let used = false;
        program.traverse({
          MemberExpression(path) {
            const { object, property, computed } = path.node;
            if (computed || !t.isIdentifier(property, { name: "hot" })) return;
            if (!t.isMetaProperty(object) || object.meta.name !== "import" || object.property.name !== "meta") return;
            path.replaceWith(t.identifier("__nd_hot"));
            used = true;
          },
        });
        if (!used) return;
        program.unshiftContainer(
          "body",
          t.variableDeclaration("const", [
            t.variableDeclarator(
              t.identifier("__nd_hot"),
              t.callExpression(t.memberExpression(t.identifier("globalThis"), t.identifier("__nd_solid_hot")), [t.stringLiteral(id)]),
            ),
          ]),
        );
      },
    },
  };
}

async function compileJsx(path: string): Promise<string> {
  let source = await Bun.file(path).text();
  let inputSourceMap: TransformOptions["inputSourceMap"];
  if (refreshCompiler) {
    const refreshed = await refreshCompiler.transformRefreshAsync(source, {
      filename: path,
      bundler: "vite",
      jsx: false,
      fixRender: false,
      importSource: "solid-js/refresh",
      sourceMap: true,
    });
    source = refreshed.code;
    if (refreshed.map) inputSourceMap = JSON.parse(String(refreshed.map));
  }
  const out = await transformAsync(source, {
    filename: path,
    babelrc: false,
    configFile: false,
    sourceMaps: "inline",
    inputSourceMap,
    plugins: dev ? [hotContext(path)] : [],
    presets: [
      [presetTypescript, { isTSX: true, allExtensions: true }],
      [presetSolid, { generate: "universal", moduleName: "@nativedesktop/solid" }],
    ],
  });
  if (!out?.code) throw new Error(`babel-preset-solid produced no output for ${path}`);
  return out.code;
}

const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const filters = [serverBuild.source, jsx.source, ...pinnedDirs.map((d) => `^${escape(d)}.*\\.[cm]?[jt]s$`)];

Bun.plugin({
  name: "nativedesktop-solid",
  setup(build) {
    build.onLoad({ filter: new RegExp(filters.join("|")) }, async (args) => {
      const path = args.path;
      if (serverBuild.test(path)) return { contents: `export * from ${JSON.stringify(clientBuild)};`, loader: "js" };
      const redirect = devRedirects.get(path);
      if (redirect) return { contents: `export * from ${JSON.stringify(redirect)};`, loader: "js" };
      if (isPinned(path)) {
        const ns = globalThis.__nd_solid_pins!.get(path);
        if (ns) return { contents: facade(path, ns), loader: "js" };
        const loader = /\.[cm]?ts$/.test(path) ? "ts" : "js";
        return { contents: selfPin(path, await Bun.file(path).text()), loader };
      }
      if (jsx.test(path)) {
        if (isSolidSource(path)) return { contents: await compileJsx(path), loader: "js" };
        return { contents: await Bun.file(path).text(), loader: path.endsWith(".jsx") ? "jsx" : "tsx" };
      }
      // register.ts itself, matched by its directory's filter.
      return { contents: await Bun.file(path).text(), loader: "ts" };
    });
  },
});
