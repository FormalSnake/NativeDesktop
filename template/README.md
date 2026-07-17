# nativedesktop-app

Scaffolded from the NativeDesktop template.

## Linking `@nativedesktop/react`, `@nativedesktop/host`, and `nd`

`package.json` references `@nativedesktop/react` and `nd` (which in turn depends on
`@nativedesktop/host`) as `file:` paths into the framework checkout's `packages/`. None of these are
published to npm yet, so this app must live alongside (or be scaffolded with a path
pointing into) a NativeDesktop checkout. `scripts/new-app.sh` rewrites every `file:../packages/*`
path to the correct relative location for the scaffolded destination (see "`bun create` vs
`scripts/new-app.sh`" below). Run `bun install` after scaffolding to materialize the dependencies.

`@nativedesktop/react` declares `react` as a `peerDependency` instead of a
regular `dependency`, so Bun hoists a single shared `react` install for this app and the linked
package. That closes the two-copies "Invalid hook call" failure a `postinstall` dedupe script used
to work around; `scripts/dedupe-react.mjs` and its `postinstall` hook are deleted.

## Running

```
bun run dev   # == nd dev  (src/main.tsx, ND_DEV=1, hot reload + crash-restart overlay)
```

`nd dev [entry]` (`packages/nd`) resolves the native host binary for your platform — the AppKit
shell on macOS, the GTK host on Linux, overridable with `--backend gtk|appkit` or `ND_BACKEND` — via
`@nativedesktop/host`'s `resolveHostBinary()` and spawns it with `ND_DEV=1 ND_SCRIPT=<entry>`;
`entry` defaults to `src/main.tsx`. If `nativedesktop.config.ts` declares app-owned native plugins,
`nd` first runs only their cached app build commands and passes their shared-library paths to that
same prebuilt host; it never rebuilds NativeDesktop. See `docs/native-components.md` in the
framework checkout and `native/README.md` in this app. `ND_DEV=1` is the underlying mechanism: it runs the Bun child
under `--hot` and enables the in-window crash-restart button. `nd dev` does not set
`NATIVE_AUTOMATION=1` itself; export it in your shell first if you need the automation socket. If
you're iterating on the framework's Zig or Swift host rather than this app's code, invoke the raw
form directly against your freshly built host binary instead, since `nd dev` prefers the prebuilt
binary resolved by `@nativedesktop/host`:

```
ND_DEV=1 ND_SCRIPT=src/main.tsx <path-to-nd-host-binary>
```

## React Compiler

React Compiler support is opt-in and off by default. `babel-plugin-react-compiler@1.0.0`
runs cleanly as a build pre-pass over `src/`, and the compiled output runs correctly against
`@nativedesktop/react` (verified headless: 3 clicks, label updated, screenshot captured; see
`docs/agents/README.md`). It is a pre-pass, not inline, because Bun's runtime transpiler does not
run babel plugins; `bun --hot` re-evaluates modules through Bun's own transpiler only. Run it with:

```
bun run compile   # babel src -> dist, react-compiler pass + JSX-to-calls, then run dist/main.tsx
```

`babel.config.json` runs three plugins in one pass: `babel-plugin-react-compiler` (the memoization
transform), `@babel/plugin-transform-react-jsx` (JSX to `@nativedesktop/react/jsx-runtime` calls,
which avoids Bun's undocumented, version-fragile dev-vs-prod jsx-runtime pragma selection
entirely since the compiled output contains no JSX syntax left for Bun to transform), and
`babel-plugin-nativedesktop` (rewrites named hook imports `from "react"` to `from
"@nativedesktop/react"` so shared hooks written the normal way for web/React Native still resolve to
the pinned instance in the compiled output; see `docs/agents/README.md`'s hook-rewrite section).
`dist/` is not part of the dev loop: `bun run dev`/`nd dev` (`ND_DEV=1` + `--hot`) still points at
uncompiled `src/`, so hot reload and react-refresh are unaffected. `nd build` (== `bun run compile`)
only compiles to `dist/`; run the compiled output with `bun run compile && ND_SCRIPT=dist/main.tsx
<path-to-nd-host-binary>` for a compiled production run.

## `bun create` vs `scripts/new-app.sh`

`bun create` does not accept an arbitrary relative path:
`bun create ./template <dest>` falls through to trying `bunx create-template` against npm, since
Bun's CLI only treats `./.bun-create/<name>` or `$HOME/.bun-create/<name>` as local
templates. `bun create <name> <dest>` does work once the template is copied to
`./.bun-create/<name>` first, but that's an extra manual step with no name-rewriting or
`docs/agents/*` seeding. `scripts/new-app.sh <dest>` is the primary
scaffolder: it copies `template/`, rewrites the app name, seeds `docs/agents/*`, and fixes up
every `@nativedesktop`-family `file:` path (`@nativedesktop/react`, `@nativedesktop/native`, `nd`, and
`nd`'s own `@nativedesktop/host` dependency) in one step. Use `bun create` only if you've already staged
the template under `.bun-create/` yourself.
