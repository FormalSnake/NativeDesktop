# Packaging + updates

`nd package` turns an app directory (a `nativedesktop.config.ts` + `package.json`)
into a distributable native bundle: a deep-signed `.app` on macOS, an
AppImage/AppDir on Linux. `nd doctor` checks the toolchain and config before you
try. The implementation lives in `packages/nd/src/package/`;
`bun tools/package.ts <mac|linux>` remains as a shim that packages the gallery
example through it (the M9 gates and `package.yml` call it that way).

## Commands

```bash
nd package               # package for the host platform (mac on darwin, linux on linux)
nd package mac           # <Name>.app, deep-signed, optional notarize + update archive
nd package linux         # AppDir + AppImage (mksquashfs fallback) + optional update archive
nd doctor [--json]       # readiness checks; non-zero exit only on real gaps

nd package [mac|linux] [--out <dir>] [--entry <file>] [--version <v>] [--cwd <dir>]
           [--no-compile] [--sign <identity>|--no-sign] [--notarize|--no-notarize]
           [--format appimage|appdir]
```

Windows packaging lands with M7; `nd package windows` exits 2 with usage.
`ND_APP_VERSION` and `--version` override the configured version.

Stdout markers (greppable, kept stable for the gates): `ND_PACKAGE_APP_SIGNED`,
`ND_PACKAGE_NOTARIZE_OK|SKIPPED`, `ND_PACKAGE_APPIMAGE`,
`ND_PACKAGE_MANIFEST <path> pub=<key>`, `ND_PACKAGE_OK <path>`,
`ND_PACKAGE_UPDATES_SKIPPED reason=not-configured`,
`ND_PACKAGE_ICON_SKIPPED reason=no-resizer`.

## Configuration

Everything is driven by `nativedesktop.config.ts`:

```ts
export default defineConfig({
  app: {
    id: "com.example.myapp",        // reverse-DNS; required for icons/mime/updates
    name: "MyApp",                  // <Name>.app, usr/bin/<slug>. Default: package.json name
    displayName: "My App",          // CFBundleDisplayName / .desktop Name
    version: "1.0.0",               // default: package.json version, then "0.0.0"
    icon: { source: "assets/icon.png" },   // or a string, or { macos, linux }
    categories: ["Utility"],        // .desktop Categories=
    fileAssociations: [{ ext: "md", name: "Markdown", mimeType: "text/markdown" }],
    urlSchemes: [{ scheme: "myapp" }],
  },
  package: {
    entry: "src/main.tsx",          // default
    compile: "auto",                // run the app's `compile` script when declared; false ships raw source
    workspaceRoot: ".",             // e.g. "../.." in a monorepo: the bundle app root mirrors it
    include: [],                    // extra workspace-relative dirs copied into the bundle
    runtimeDependencies: [],        // extra roots for the module-graph walk
    outDir: "dist",                 // resolved against workspaceRoot
    mac: { minimumSystemVersion: "26.0", category: "public.app-category.utilities" },
    linux: { format: "appimage" },
    updates: { baseUrl: "https://updates.example.com/myapp" },   // opt-in; omit for no updater
  },
});
```

`workspaceRoot` is the one knob for monorepos: the bundle's app root mirrors the
workspace, so the app's own files land at their workspace-relative path and
relative imports/spawns keep resolving. The gallery example uses
`workspaceRoot: "../.."` so it lands at `app/examples/gallery` inside the bundle.

## What a bundle contains

- The resolved host binary from `@nativedesktop/host` (`Contents/MacOS/<Name>`
  on mac, `usr/bin/<slug>` on linux) and the Bun runtime next to it.
- The app payload under `Resources/app` / `AppDir/app`: the compiled outDir
  (or the entry's source dir; a root-level entry ships the whole app dir), the
  app's `package.json`, every `include` path, and built native plugins under
  `app/native/`.
- A flat `app/node_modules`: `nd package` walks the real runtime module graph
  (app `dependencies` + `runtimeDependencies`, then each package's
  `dependencies`, resolvable `optionalDependencies`/`peerDependencies`) and
  copies every package into one flat tree that an ordinary upward walk resolves.
  This subsumes Bun's isolated store, workspace symlinks, and `file:` deps;
  there is no `bun install` inside the bundle. Version conflicts nest under the
  requiring package. Each copied package's entry must resolve on disk, so an
  unbuilt `dist/` fails packaging loudly instead of shipping broken.
- `app/nd-app.json`: `{ id, name, version, entry, cwd, pluginPaths, engine,
  schemes }` (entry/cwd app-root-relative), the packaged-launch contract below.

## Packaged apps launch by themselves

- macOS: `NDBundleBootstrap` (swift/Sources/NDShell/BundleBootstrap.swift) runs
  at startup. When `Resources/app/nd-app.json` exists and no explicit
  `ND_SCRIPT` is set, it points `ND_SCRIPT` at the bundled entry, prepends
  `Contents/MacOS` to `PATH` (the bundled `bun`), chdirs to `app/<cwd>` (so
  `getAppDataDir()` and relative fs reads behave the same packaged as in dev),
  and exports `ND_PLUGINS`/`ND_PLUGIN_PATHS` for bundled plugins. An explicit
  `ND_SCRIPT` (dev override, gate scripts) wins wholesale: script, cwd, and PATH
  are left alone. The manifest's `engine`/`schemes` are exported earlier still
  (`applyEngine()`, before CEF is prepared) and stand whatever `ND_SCRIPT` says,
  since they describe what is staged in the bundle; an explicit
  `ND_WEBVIEW_ENGINE`/`ND_CEF_SCHEMES` still wins.
- Linux: the generated `AppRun` bakes the same values (absolute `ND_SCRIPT`,
  `cd`, `PATH`, `ND_APP_ID=<app.id>`, plugin paths, and
  `ND_WEBVIEW_ENGINE`/`ND_CEF_SCHEMES` behind a `${VAR:-…}` override). The gtk
  host reads `ND_APP_ID` for its GApplication id, and the generated `.desktop`
  carries `StartupWMClass=<app.id>` so icon and window grouping bind.

## Icons

- macOS: `.icns` and `.iconset` sources pass through; a PNG is resized with
  `sips` into an iconset and compiled with `iconutil`. An SVG-only source is a
  hard error (supply a 1024px PNG or a prebuilt `.icns`).
- Linux: an SVG installs under `usr/share/icons/hicolor/scalable/apps/`; a PNG
  is resized into the hicolor sizes with whichever of
  `sips`/`magick`/`convert`/`rsvg-convert` exists, plus `<slug>.png` at the
  AppDir root (appimagetool requires it). With no resizer the source installs
  at its native size only and `ND_PACKAGE_ICON_SKIPPED reason=no-resizer` is
  printed.
- No icon configured: the AppDir keeps a 1x1 placeholder and a one-time warning
  is printed.

## macOS signing + notarization

Deep-sign runs inside-out: `bun`, the host binary, bundled plugin dylibs, then
`--deep` on the `.app`, then `codesign --verify --strict`. The hardened runtime
and `com.apple.security.cs.allow-jit` entitlements apply to every nested Mach-O
(`bun` is the process that needs allow-jit for JSC on Apple Silicon).

Identity resolution: `--sign <identity>` wins, then `package.mac.signIdentity`,
then `APPLE_SIGN_IDENTITY`, else ad-hoc (`-`). Ad-hoc-signed apps launch locally
and pass `codesign --verify`; that is "runs here", not "safe to distribute".
`--no-sign` skips codesign entirely.

Notarization (`xcrun notarytool submit` + `stapler staple`) runs when `APPLE_ID`,
`APPLE_TEAM_ID`, and `APPLE_APP_PASSWORD` are all set, or when forced with
`--notarize` (which errors without credentials). Otherwise
`ND_PACKAGE_NOTARIZE_SKIPPED reason=no-credentials` is printed; that is the
expected, asserted path in this repo's gates.

A custom `package.mac.infoPlist` file is used verbatim as the base plist, with
the identity stamped in (CFBundleIdentifier, CFBundleExecutable, version keys,
document types, URL schemes); by default the plist is generated in-code.

## Updates are opt-in

Without `package.updates`, no archive or manifest is produced
(`ND_PACKAGE_UPDATES_SKIPPED reason=not-configured`) and `nd doctor` warns that
the shipped app has no updater. With it, packaging produces a signed
full-archive payload (`.tar.gz` mac / `.tar.zst` linux, overridable via
`updates.format`) under `<outDir>/update/` plus a minisign-signed manifest.

Key resolution: `updates.secretKey`/`publicKey`, else
`ND_MINISIGN_SEC`/`ND_MINISIGN_PUB`, else `ephemeralKey: true` generates a
throwaway pair (CI/test path only; the gallery config uses it). `minisign` must
be on PATH (`brew install minisign` on a Mac).

### Manifest schema

Produced by `packages/nd/src/package/updates.ts` (`buildAndSignManifest`),
consumed by `src/core/update.zig`'s `parseManifest`:

```json
{
  "app_id": "com.nativedesktop.gallery",
  "version": "0.9.0",
  "from": null,
  "full_url": "https://updates.example.com/gallery-0.9.0-linux.tar.zst",
  "full_sig_b64": "<base64 algo+key_id+signature blob>"
}
```

`full_sig_b64` is the second base64 line of the archive's `.minisig`
(`algorithm[2] | key_id[8] | signature[64]`), passed through verbatim for the
Zig verifier. `from`/`delta` stay reserved for future delta artifacts; only
full archives ship today (M9-D2).

### Verification is non-disableable

The minisign/Ed25519 verifier (`src/core/update.zig`, `verifyMinisign`) is a
pure function with zero I/O, and every consumer (`zig build update-verify`,
`scripts/m9-drive.ts`) calls it unconditionally before staging anything. No
flag, environment variable, or manifest field skips it. The whole flow is
tested against a loopback server (`tools/update-server.ts`); nothing depends on
a remote host at test time.

## Linux notes

The AppDir is packed with `appimagetool` when available, else `mksquashfs`
(the nix devshell case), else packaging fails at the packing step;
`--format appdir` stops at the raw AppDir. A committed Flatpak manifest lives
at `packaging/flatpak/com.nativedesktop.gallery.yml` and is only lint-validated
in CI (a full `flatpak-builder` run needs a real GNOME runner).

## Gates

`scripts/headless-m9.sh` (Linux: package, launch the packaged AppDir binary
under weston, run the update flow) and `scripts/mac/mac-m9.sh` (macOS: package,
verify the ad-hoc signature + allow-jit, launch the packaged `.app`, run the
update flow). `packageApp` cleans `<outDir>/<platform>` itself before
assembling, so repeat runs need no manual `rm -rf`.
