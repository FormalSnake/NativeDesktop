# nativedesktop-app

Scaffolded from the NativeDesktop template.

## Linking `@nativedesktop/react`

`package.json` references `@nativedesktop/react` as a `file:` path into the framework checkout's
`packages/react`. The package isn't published to npm yet (that lands with M9 packaging) — until then,
this app must live alongside (or be scaffolded with a path pointing into) a NativeDesktop checkout. Run
`bun install` after scaffolding to materialize the dependency.

**Known limitation, worked around automatically:** because the linked package stays at its real
location inside the framework checkout, and `packages/react` currently declares `react`/
`react-reconciler` as regular dependencies (not `peerDependencies`), Node/Bun module resolution can
end up loading two separate copies of `react` — one for this app, one for the linked package —
which breaks React's hooks dispatcher ("Invalid hook call"). A `postinstall` script
(`scripts/dedupe-react.mjs`) re-points this app's own `node_modules/react` and
`node_modules/react-reconciler` at the exact copies the linked package resolves, so only one copy of
each ever loads. This is a workaround, not a real fix — once `packages/react` declares `react` as a
peer dependency (or the package is npm-published in M9), this script and the `postinstall` hook can
be deleted.

## Running

There is no packaged `nd` CLI yet (that's M9 — see `docs/agents/README.md`). Run the app directly
against the framework's host binary:

```
ND_DEV=1 ND_SCRIPT=src/main.tsx <path-to-nd-host-binary>
```

`ND_DEV=1` is what will eventually be `nd dev`'s job: it runs the Bun child under `--hot` and enables the
in-window crash-restart button. Leaving it unset runs the clean/production path.

## React Compiler

**Opt-in, working, off by default.** Verified this session: `babel-plugin-react-compiler@1.0.0`
runs cleanly as a build pre-pass over `src/` and the compiled output runs correctly against
`@nativedesktop/react` (headless-driven: 3 clicks, label updated, screenshot captured — see
`docs/agents/README.md`). It is a pre-pass, not inline, because Bun's runtime transpiler does not
run babel plugins — `bun --hot` re-evaluates modules through Bun's own transpiler only (see the M8
`--hot` findings). Run it with:

```
bun run compile   # babel src -> dist, react-compiler pass + JSX-to-calls, then run dist/main.tsx
```

`babel.config.json` runs two plugins in one pass: `babel-plugin-react-compiler` (the memoization
transform) and `@babel/plugin-transform-react-jsx` (JSX to `@nativedesktop/react/jsx-runtime` calls
— this avoids Bun's undocumented, version-fragile dev-vs-prod jsx-runtime pragma selection
entirely, since the compiled output contains no JSX syntax left for Bun to transform). `dist/` is
not part of the dev loop — `bun run dev` (`ND_DEV=1` + `--hot`) still points at `src/`, uncompiled,
so hot reload and react-refresh are unaffected. Use `bun run compile && ND_SCRIPT=dist/main.tsx
<path-to-nd-host-binary>` for a compiled production run.

## `bun create` vs `scripts/new-app.sh`

**Verified this session:** `bun create` does *not* accept an arbitrary relative path
(`bun create ./template <dest>` falls through to trying `bunx create-template` against npm, since
Bun's CLI only treats `./.bun-create/<name>` or `$HOME/.bun-create/<name>` as "local" templates —
not an arbitrary directory). `bun create <name> <dest>` does work once the template is copied to
`./.bun-create/<name>` first, but that's an extra manual step with no name-rewriting or
`docs/agents/*` seeding. **`scripts/new-app.sh <dest>` is the documented, verified, primary
scaffolder** — it copies `template/`, rewrites the app name, seeds `docs/agents/*`, and fixes up the
`@nativedesktop/react` `file:` path, all in one step. Use `bun create` only if you've already staged
the template under `.bun-create/` yourself.
