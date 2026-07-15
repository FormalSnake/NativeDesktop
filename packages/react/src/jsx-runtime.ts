// The JSX intrinsic types and the jsx/jsxs/Fragment runtime re-exports are
// GENERATED from schema/widgets.json — see tools/codegen.ts. This module is a
// stable re-export point so jsxImportSource=@nativedesktop/react keeps
// resolving JSX.IntrinsicElements from the package (not from @types/react,
// whose HTML tag names would collide).
//
// JSX is re-exported as `export type` (not a value export): it is a
// TypeScript namespace containing only types (IntrinsicElements etc.), so it
// compiles to nothing at runtime. A plain `export { ..., JSX }` re-export
// only works when a type-aware transpiler (tsc, Bun compiling a source .tsx
// directly) elides it; a syntax-only transform (Babel, or Bun evaluating
// this file as a plain import target rather than inlining a JSX pragma)
// cannot tell JSX has no runtime value and crashes with "export 'JSX' not
// found" — e.g. a babel-plugin-react-compiler pre-pass, which emits a
// hand-authored `import { jsx } from "@nativedesktop/react/jsx-runtime"`.
export { jsx, jsxs, Fragment } from "./generated/intrinsics.ts";
export type { JSX } from "./generated/intrinsics.ts";
