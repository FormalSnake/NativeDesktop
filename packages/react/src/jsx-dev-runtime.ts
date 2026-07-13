// Dev-mode JSX runtime — the parallel of jsx-runtime.ts for the automatic
// dev transform. Bun (and tsc with jsx=react-jsxdev) emit jsxDEV imports from
// `@nativedesktop/react/jsx-dev-runtime` whenever the dev runtime is selected
// — which Bun does whenever the cwd holds a tsconfig.json, i.e. when a
// scaffolded app runs `nd dev` from its own directory. Mirrors
// generated/intrinsics.ts's own `export { jsx, jsxs } from "react/jsx-runtime"`:
// delegate the factory to React's dev runtime, override only JSX.IntrinsicElements.
export { jsxDEV, Fragment } from "react/jsx-dev-runtime";
export type { JSX } from "./generated/intrinsics.ts";
