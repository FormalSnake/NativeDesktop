// Bun plugin — the `bun --hot` dev-path twin of the babel plugin. Loaded via
// a `bunfig.toml` `preload` so it registers once per process (surviving hot
// re-evals) and rewrites shared `.ts` source as Bun loads it.
//
// This is NOT the module-aliasing mechanism the docs reject: onLoad here never
// touches the "react" module (Bun's resolver loads bare specifiers directly,
// so onLoad does not fire for them). It fires for the IMPORTING source files
// and edits the import specifier STRING inside them — a plain source
// transform, so no ESM/CJS `provideFetch` conflict arises. See
// docs/agents/README.md "HMR: what actually preserves state" and
// packages/react/src/dev-react.ts.
//
// FILTER IS `.ts` ONLY — deliberately excludes `.tsx`/`.desktop.tsx`. Verified
// empirically (M8-D8): Bun's runtime onLoad has no fall-through — a matched
// file MUST return contents (returning undefined or a contents-less object
// both error), and once a plugin returns contents for a file, `bun --hot`
// drops that file from its watch set, so the file no longer hot-reloads.
// Intercepting component `.tsx` files would therefore break their HMR. So we
// intercept only shared, non-component `.ts` modules — the natural home of
// cross-platform hooks (`import { useState } from "react"`). Such a `.ts` file
// is pinned at first eval: editing it requires a restart to take effect, but a
// `.tsx`/`.desktop.tsx` component that consumes it hot-reloads normally, and a
// component re-eval never resurfaces the dispatcher crash because the shared
// module's hooks stay bound to `@nativedesktop/react`. Desktop `.tsx`
// components import hooks from `@nativedesktop/react` (or from shared `.ts`
// hooks), never raw "react". The babel plugin (compiled path) has no watcher
// to preserve, so it rewrites every extension.
const { rewriteReactHookImports } = require("./rewrite.js");

const DEBUG = process.env.ND_HOOK_REWRITE_DEBUG === "1";

Bun.plugin({
  name: "nativedesktop-hook-imports",
  setup(build) {
    build.onLoad({ filter: /\.ts$/ }, async (args) => {
      const source = await Bun.file(args.path).text();
      // Dependency source is returned verbatim (never skipped — onLoad cannot
      // decline). Deps don't hot-reload anyway, so freezing them is harmless.
      const skip = args.path.includes("/node_modules/") || args.path.includes("/dist/");
      const contents = skip ? source : rewriteReactHookImports(source);
      if (DEBUG) {
        const state = skip ? "skip" : contents === source ? "nochange" : "rewrote";
        console.error(`ND_HOOK_REWRITE ${state} path=${args.path}`);
      }
      return { contents, loader: args.loader };
    });
  },
});
