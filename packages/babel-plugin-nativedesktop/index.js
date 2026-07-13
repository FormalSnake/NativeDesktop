// babel-plugin-nativedesktop — compiled/production path (M8-D8).
//
// Rewrites named React-hook imports `from "react"` to `from
// "@nativedesktop/react"` so shared, platform-agnostic hooks (authored the
// normal way, `import { useState } from "react"`, because web/React-Native
// also consume them) resolve to the HMR-pinned, reconciler-attached react
// instance when compiled into a NativeDesktop app. Only the hook subset
// dev-react.ts pins is moved; default/namespace/type-only specifiers stay on
// "react". See hooks.js for the list; bun-plugin.js is the `bun --hot`
// counterpart for the dev path (babel does not run under `bun --hot`).
const { PINNED_HOOKS } = require("./hooks.js");

const HOOK_SET = new Set(PINNED_HOOKS);
const ND_SOURCE = "@nativedesktop/react";

module.exports = function babelPluginNativedesktop({ types: t }) {
  return {
    name: "nativedesktop-hook-imports",
    visitor: {
      ImportDeclaration(path) {
        const node = path.node;
        if (node.source.value !== "react") return;
        // `import type { … } from "react"` — leave type-only imports alone.
        if (node.importKind === "type") return;

        const hookSpecifiers = [];
        const restSpecifiers = [];
        for (const spec of node.specifiers) {
          if (
            t.isImportSpecifier(spec) &&
            spec.importKind !== "type" &&
            t.isIdentifier(spec.imported) &&
            HOOK_SET.has(spec.imported.name)
          ) {
            hookSpecifiers.push(spec);
          } else {
            restSpecifiers.push(spec);
          }
        }
        if (hookSpecifiers.length === 0) return;

        const ndImport = t.importDeclaration(hookSpecifiers, t.stringLiteral(ND_SOURCE));
        if (restSpecifiers.length > 0) {
          node.specifiers = restSpecifiers;
          path.insertAfter(ndImport);
        } else {
          path.replaceWith(ndImport);
        }
      },
    },
  };
};
