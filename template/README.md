# nativedesktop-app

Scaffolded from the NativeDesktop template. Your UI lives in `src/`, is written in SolidJS, renders
to real native widgets, and runs on GTK4 with libadwaita on Linux or AppKit on macOS.

## Run it

```bash
bun install
bun run dev
```

`bun run dev` is `nd dev`. It resolves the native host binary for your platform through
`@nativedesktop/host` (the AppKit shell on macOS, the GTK host on Linux) and spawns it with
`ND_DEV=1 ND_SCRIPT=src/main.tsx`. That gives you hot reload and the in-window crash-restart overlay.

| Command | What it does |
|---|---|
| `nd dev [entry]` | Dev mode. `entry` defaults to `src/main.tsx`. |
| `nd dev --backend gtk\|appkit` | Force a backend. Also reads `ND_BACKEND`. |
| `nd build` | Compile `src/` into `dist/main.js`, the same as `bun run compile`. |
| `nd package [mac\|linux]` | Assemble and sign the platform bundle (`.app` / AppImage). Platform defaults to the host. Also `bun run package`. |
| `nd doctor [--json]` | Check packaging and toolchain readiness for this directory. |

App identity (bundle id, name, icon, file associations, URL schemes) and packaging options live in
`nativedesktop.config.ts`; `nd package` reads them.

`nd dev` does not set `NATIVE_AUTOMATION=1`. Export it in your shell first if you want the
automation socket.

If `nativedesktop.config.ts` declares app-owned native plugins, `nd` runs their cached build
commands first and passes the resulting shared-library paths to the prebuilt host. It never rebuilds
NativeDesktop itself. See `docs/native-components.md` in the framework checkout and `native/README.md`
here. `defineNativeComponent` from `@nativedesktop/solid` gives such a view a typed component.

When you are iterating on the framework's own Zig or Swift host rather than this app, invoke the raw
form against your freshly built binary, since `nd dev` prefers the prebuilt one. The Solid JSX
transform is a Bun preload, so pass it yourself:

```bash
BUN_OPTIONS=--preload=@nativedesktop/solid/register ND_DEV=1 ND_SCRIPT=src/main.tsx <path-to-nd-host-binary>
```

## Writing components

Components are Solid components: each runs once, and the JSX expressions that read signals update
the native widgets they feed. Read a signal by calling it (`clicks()`) inside JSX, a memo or an
effect, and pass props as values (`<Panel open={open()} />`). Refs are plain variables or
callbacks: `let win: NdNodeRef<"window"> | undefined` with `<window ref={win}>`.

Primitives (`createSignal`, `createMemo`, `For`, `Show`, `Errored`, `Loading`) come from
`solid-js`; the widgets, `render` and the platform APIs come from `@nativedesktop/solid`.
`src/hooks/useToggle.ts` is plain `solid-js` with no JSX, so the same file also works in a web Solid
app.

Under `nd dev` an edit to a component patches it in place: the window and the state of the
components you did not edit stay.

## How this app links to the framework

`package.json` depends on the published npm packages: `@nativedesktop/solid` (the renderer and its
Bun preload), `solid-js`, `@nativedesktop/native` (native-plugin headers), and `@nativedesktop/cli`
(the `nd` bin, which pulls in `@nativedesktop/host` and the prebuilt host binary for your platform).
Optional additions from the same family: `@nativedesktop/data` (worker-backed SQLite, with
`createQuery` at `@nativedesktop/data/solid`), `@nativedesktop/rpc` (JSON-RPC client for your own
services), and `@nativedesktop/test` (automation harness for scripted app tests).

When scaffolded from a framework checkout, `scripts/new-app.sh` rewrites those registry versions to
`file:` paths into the checkout so the app exercises your local build instead of npm.

## Errors and settings

Two framework defaults worth knowing from day one:

- **Async errors do not kill the app by default.** An unhandled promise rejection is reported and
  the app keeps running; an uncaught exception is fatal (the host paints the crash overlay). A
  render error under an `<Errored>` boundary is reported and the boundary shows its fallback; one no
  boundary catches is fatal. Tune the async half with `setUnhandledErrorPolicy` and subscribe with
  `onUnhandledError`, both from `@nativedesktop/solid`.
- **Settings persist through `createStore`.** A versioned JSON file under the app data dir; call
  `await store.load()` before `render()` and `store.get()` is synchronous in every component, with
  `useStoreValue(store)` for a signal of it. Writes are debounced and crash-safe.

## Production build

`bun run compile` (what `nd build` and `nd package` run) is `nd-solid-build`: it bundles `src/`
into `dist/main.js` with the Solid transform already applied, so the packaged app starts without
compiling JSX. `nativedesktop.config.ts` names `dist/main.js` as the packaged entry. `dist/` is not
part of the dev loop; `nd dev` runs `src/` directly.

## Why not `bun create`

`bun create ./template <dest>` does not work: Bun only treats `./.bun-create/<name>` or
`$HOME/.bun-create/<name>` as local templates, so a relative path falls through to
`bunx create-template` against npm. `bun create <name> <dest>` works once you have copied the
template into `./.bun-create/<name>` yourself, but that skips the name rewrite, the `docs/agents/*`
seeding, and the `file:` path fixups. Use `scripts/new-app.sh <dest>`.
