// String-level twin of the babel plugin's ImportDeclaration transform, for
// the `bun --hot` dev path (bun-plugin.js) where babel never runs. Bun's
// onLoad hands us raw source text, not an AST, so this rewrites the import
// clause textually. Scoped to well-formed TS/TSX import statements: only
// `import [Default,] { … } from "react"` shapes are touched, never
// `import type …`, `import React from "react"`, or `import * as React`.
const { PINNED_HOOKS } = require("./hooks.js");

const HOOK_SET = new Set(PINNED_HOOKS);
const ND_SOURCE = "@nativedesktop/react";

// `[^{}]*` cannot cross a brace, so it can never swallow a neighbouring
// import statement's clause even when semicolons are elided (ASI). The
// `(?!type\b)` guard skips `import type { … } from "react"`.
const REACT_NAMED_IMPORT = /import\s+(?!type\b)(?:([A-Za-z_$][\w$]*)\s*,\s*)?\{([^{}]*)\}\s*from\s*(['"])react\3\s*;?/g;

function rewriteReactHookImports(code) {
  if (!code.includes('"react"') && !code.includes("'react'")) return code;
  return code.replace(REACT_NAMED_IMPORT, (match, defaultImport, named, quote) => {
    const specs = named
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);

    const hookSpecs = [];
    const restSpecs = [];
    for (const spec of specs) {
      if (/^type\s/.test(spec)) {
        restSpecs.push(spec);
        continue;
      }
      const imported = spec.split(/\s+as\s+/)[0].trim();
      if (HOOK_SET.has(imported)) hookSpecs.push(spec);
      else restSpecs.push(spec);
    }
    if (hookSpecs.length === 0) return match;

    const out = [];
    if (defaultImport && restSpecs.length) {
      out.push(`import ${defaultImport}, { ${restSpecs.join(", ")} } from ${quote}react${quote};`);
    } else if (defaultImport) {
      out.push(`import ${defaultImport} from ${quote}react${quote};`);
    } else if (restSpecs.length) {
      out.push(`import { ${restSpecs.join(", ")} } from ${quote}react${quote};`);
    }
    out.push(`import { ${hookSpecs.join(", ")} } from ${quote}${ND_SOURCE}${quote};`);
    return out.join(" ");
  });
}

module.exports = { rewriteReactHookImports };
