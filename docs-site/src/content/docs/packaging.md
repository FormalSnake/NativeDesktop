---
title: Packaging
description: nd package and nd doctor, the packaging config reference, icons, the packaged-launch contract, and opt-in signed updates.
---

`nd package` turns an app directory (a `nativedesktop.config.ts` plus a `package.json`) into a
distributable native bundle: a deep-signed `.app` on macOS, an AppImage or AppDir on Linux.
`nd doctor` checks the toolchain and config before you try.

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

Windows packaging lands with the Windows backend; `nd package windows` exits 2 with usage. See
[Platform Support](/native-platform/platform-support/). `ND_APP_VERSION` and `--version` override
the configured version.

## Configuration

```ts
export default defineConfig({
  app: {
    id: "com.example.myapp",        // reverse-DNS; required for icons/mime/updates
    name: "MyApp",                  // <Name>.app, usr/bin/<slug>. Default: package.json name
    displayName: "My App",          // CFBundleDisplayName / .desktop Name
    version: "1.0.0",               // default: package.json version, then "0.0.0"
    icon: { source: "assets/icon.png" },   // or a string, or { macos, linux, layered }
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

`workspaceRoot` is the knob for monorepos. The bundle's app root mirrors the workspace, so the app's
own files land at their workspace-relative path and relative imports and spawns keep resolving
inside the bundle. `include` copies extra workspace-relative directories, such as a sibling daemon
package or shared assets, into the same tree.

## What a bundle contains

- The resolved host binary from `@nativedesktop/host` (`Contents/MacOS/<Name>` on mac,
  `usr/bin/<slug>` on linux) and the Bun runtime next to it.
- The app payload under `Resources/app` / `AppDir/app`: the compiled outDir (or the entry's source
  dir; a root-level entry ships the whole app dir), the app's `package.json`, every `include`
  path, and built native plugins under `app/native/`.
- A flat `app/node_modules`: `nd package` walks the real runtime module graph (app `dependencies`
  plus `runtimeDependencies`, then each package's `dependencies` and resolvable
  `optionalDependencies`/`peerDependencies`) and copies every package into one flat tree an
  ordinary upward walk resolves. This subsumes Bun's isolated store, workspace symlinks, and
  `file:` deps; there is no `bun install` inside the bundle. Version conflicts nest under the
  requiring package, and each copied package's entry must resolve on disk, so an unbuilt `dist/`
  fails packaging loudly instead of shipping broken.
- `app/nd-app.json`: `{ id, name, version, entry, cwd, pluginPaths }` (entry/cwd
  app-root-relative), the packaged-launch contract below.

## Packaged apps launch by themselves

On macOS the shell's bundle bootstrap runs at startup: when `Resources/app/nd-app.json` exists and
no explicit `ND_SCRIPT` is set, it points `ND_SCRIPT` at the bundled entry, prepends
`Contents/MacOS` to `PATH` (the bundled `bun`), chdirs to `app/<cwd>` (so
[`getAppDataDir()`](/core-concepts/app-data-storage/) and relative fs reads behave the same
packaged as in dev), and exports `ND_PLUGINS`/`ND_PLUGIN_PATHS` for bundled plugins. An explicit
`ND_SCRIPT` (dev override, gate scripts) wins wholesale.

On Linux the generated `AppRun` bakes the same values, plus `ND_APP_ID=<app.id>`: the gtk host
uses it as its GApplication id, and the generated `.desktop` carries `StartupWMClass=<app.id>` so
icon and window grouping bind.

## Icons

`app.icon` takes a PNG, SVG, `.icns`, `.iconset`, or an Icon Composer `.icon` bundle, and
`app.icon.layered` describes a macOS 26 layered icon in config. Every size each platform needs is
generated at package time, and a layered composition flattens to the Linux icon so one definition
covers both. See [App Icons](/packaging/app-icons/) for the source matrix, the layered format, and
the converter degrade paths.

## File associations and URL schemes

`app.fileAssociations` and `app.urlSchemes` flow into the macOS Info.plist
(`CFBundleDocumentTypes`/`CFBundleURLTypes`) and the Linux `.desktop`'s `MimeType=` line plus a
`usr/share/mime/packages/<app.id>.xml` shared-mime-info file. macOS keys document types off the
extension, so `mimeType` is optional there; Linux needs a real `mimeType` for a file association
to register. Launches arrive via
[`app.onOpenFile`/`app.onOpenUrl`](/native-platform/system-capabilities/#app-level-events).

## macOS signing + notarization

Deep-sign runs inside-out: `bun`, the host binary, bundled plugin dylibs, then `--deep` on the
`.app`, then `codesign --verify --strict`. The hardened runtime and
`com.apple.security.cs.allow-jit` entitlements apply to every nested Mach-O (`bun` is the process
that needs allow-jit for JSC on Apple Silicon).

Identity resolution: `--sign <identity>` wins, then `package.mac.signIdentity`, then
`APPLE_SIGN_IDENTITY`, else ad-hoc (`-`). An ad-hoc-signed `.app` launches locally and passes
`codesign --verify`, which means it runs on this machine, not that it is safe to distribute.
`--no-sign` skips codesign entirely.

Notarization (`xcrun notarytool submit` + `stapler staple`) runs when `APPLE_ID`,
`APPLE_TEAM_ID`, and `APPLE_APP_PASSWORD` are all set, or when forced with `--notarize` (which
errors without credentials). Otherwise packaging prints
`ND_PACKAGE_NOTARIZE_SKIPPED reason=no-credentials` and continues; that is expected behavior, not
a failure.

A custom `package.mac.infoPlist` file is used verbatim as the base plist, with the identity
stamped in (CFBundleIdentifier, CFBundleExecutable, version keys, document types, URL schemes); by
default the plist is generated in-code.

## Updates are opt-in

Without `package.updates`, no archive or manifest is produced
(`ND_PACKAGE_UPDATES_SKIPPED reason=not-configured`) and `nd doctor` warns that the shipped app
has no updater. With it, packaging produces a signed full-archive payload (`.tar.gz` mac /
`.tar.zst` linux, overridable via `updates.format`) under `<outDir>/update/` plus a
minisign-signed manifest, `full_url` pointing at `<baseUrl>/<archive-basename>`.

Key resolution: `updates.secretKey`/`publicKey`, else `ND_MINISIGN_SEC`/`ND_MINISIGN_PUB`, else
`ephemeralKey: true` generates a throwaway pair (CI/test path only). `minisign` must be on PATH
(`brew install minisign` on a Mac).

### Manifest schema

Produced by `buildAndSignManifest` (`packages/nd/src/package/updates.ts`), consumed by
`src/core/update.zig`'s `parseManifest`:

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
(`algorithm[2] | key_id[8] | signature[64]`), passed through verbatim for the Zig verifier.
`from`/`delta` stay reserved for future delta artifacts; only full archives ship today.

### Verification is non-disableable

The minisign/Ed25519 verifier (`src/core/update.zig`, `verifyMinisign`) is a pure function with
zero I/O, and every consumer calls it unconditionally before staging anything. No flag,
environment variable, or manifest field skips it. The whole flow is tested against a loopback
server (`tools/update-server.ts`); nothing depends on a remote host at test time.

## Linux notes

The AppDir is packed with `appimagetool` when available, else `mksquashfs` (the nix devshell
case); `--format appdir` stops at the raw AppDir. A committed Flatpak manifest lives at
`packaging/flatpak/com.nativedesktop.gallery.yml` and is only lint-validated in CI; a full
`flatpak-builder` run needs a real GNOME runner.
