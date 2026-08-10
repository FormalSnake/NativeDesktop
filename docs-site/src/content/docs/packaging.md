---
title: Packaging
description: nd package, the update manifest format, and how the auto-update flow verifies what it downloads.
---

`nd package <platform>` is a documented convention rather than a shipped subcommand, like
`nd codegen` (which means `bun tools/codegen.ts`). The `bin/nd` dispatcher (`packages/nd`, see
[Project Layout](/get-started/project-layout/)) implements `nd dev` and `nd build` for
running and compiling an app; `nd package <platform>` means running `bun tools/package.ts
<platform>` directly.

## Commands

```bash
bun tools/package.ts linux   # AppImage (or squashfs fallback) + signed .tar.zst update
bun tools/package.ts mac     # Gallery.app (deep-signed) + signed .tar.gz update
```

`ND_APP_VERSION` (default `0.9.0`) sets the packaged version and the version recorded in the update
manifest.

Windows packaging (signed NSIS installer + winget manifest) lands with the Windows backend, which
is not yet implemented. `tools/package.ts` exits with an error and a usage message for any
platform argument other than `linux`/`mac`. See [Platform Support](/native-platform/platform-support/).

## Linux: AppImage + Flatpak

`tools/package-linux.ts` assembles a self-contained AppDir under `dist/linux/AppDir` from
`packaging/AppDir.template/` (`.desktop`, icon, `AppRun`), the built `nd-hello` binary, the Bun
runtime, and the app sources (re-`bun install --production`'d inside the AppDir). It packs the
AppDir into an AppImage with `appimagetool`, and falls back to `mksquashfs` if `appimagetool` isn't
on `PATH`. The nix devshell doesn't include `appimagetool`, so CI and local runs exercise the
squashfs fallback path.

A committed Flatpak manifest exists at `packaging/flatpak/com.nativedesktop.gallery.yml` (GNOME
50 runtime, `nd-hello` command, wayland/fallback-x11/dri finish-args only; no portal permissions,
since in-process automation needs none). The CI gate lint-validates this manifest
(`flatpak-builder --show-manifest packaging/flatpak/com.nativedesktop.gallery.yml`) but does not run
a full `flatpak-builder` build, which is fragile inside a nix sandbox or stock CI (portal, runtime,
and bubblewrap requirements). The full build is deferred to a real GNOME runner.

## macOS: `.app` + codesign + notarization

`tools/package-mac.ts` assembles `Gallery.app` around `swift/.build/release/NDShell`, the bundled
Bun runtime, and the app sources, then deep-signs inside-out, nested Mach-O binaries (`bun`,
`NDShell`) before the `.app` itself, with the hardened runtime (`--options runtime`) and a
`com.apple.security.cs.allow-jit` entitlement (JSC-under-Bun needs it on Apple Silicon).

If `APPLE_SIGN_IDENTITY` is set, it's used as the `codesign -s` identity;
otherwise packaging falls back to ad-hoc signing (`codesign -s -`).

`--options runtime` is accepted alongside an ad-hoc
signature, and the entitlements are embedded, but the OS only enforces hardened-runtime
protections for signatures backed by a valid Team ID. An ad-hoc-signed `.app` launches locally
and passes `codesign --verify`, which is all the packaging gate asserts. That is not the same
guarantee a Team-ID signature gives; treat ad-hoc as "runs here", not "safe to distribute".

Notarization runs when credentials are present: `xcrun notarytool submit` and
`xcrun stapler staple` run only when `APPLE_ID`, `APPLE_TEAM_ID`, and `APPLE_APP_PASSWORD` are all
set in the environment. This repo's dev and CI environments have no such credentials, so
packaging takes the skip path and prints `ND_PACKAGE_NOTARIZE_SKIPPED reason=no-credentials`.
That's expected behavior, not a failure.

The Mac packaging flow signs update archives with `minisign`, which isn't
part of Xcode or the system toolchain; install it with `brew install minisign`.

## File associations and URL schemes

`nd package` stamps an app's identity (a bundle id override, document types, and custom URL
schemes) into the platform packaging outputs, driven by the `app` field of
`nativedesktop.config.ts` (`AppIdentity`, `packages/nd/src/config.ts`):

```ts
export default defineConfig({
  app: {
    id: "com.example.myapp",
    fileAssociations: [{ ext: "md", name: "Markdown Document", mimeType: "text/markdown", role: "editor" }],
    urlSchemes: [{ scheme: "myapp" }],
  },
});
```

- **`id`** overrides `CFBundleIdentifier` on macOS. There's no Linux equivalent; the `.desktop`
  file's identity comes from its filename.
- `fileAssociations`: each entry is one document type, with `ext` (no dot), an optional readable
  `name`, an optional `mimeType`, and `role` (`"editor"` or `"viewer"`, defaulting to `"editor"`).
- `urlSchemes`: each entry is one custom scheme, `{ scheme, name? }`, for example `myapp://...`.

On macOS, `tools/app-identity.ts` injects `CFBundleDocumentTypes` (one `<dict>` per file
association) and `CFBundleURLTypes` (one per URL scheme) into `Info.plist`, alongside the
`CFBundleIdentifier` override when `id` is set. `fileAssociations` work here even without a
`mimeType`, since macOS keys document types off the extension and `UTType` rather than a MIME type.

On Linux, the same config extends the AppDir's `.desktop` entry: a `MimeType=` line built from every
file association's `mimeType` plus `x-scheme-handler/<scheme>` for each URL scheme, and an `Exec=...
%U` argument so the launched app receives the opened path/URL. A `usr/share/mime/packages/<app>.xml`
shared-mime-info file is also generated for any file association that declares a `mimeType`. A file
association without a `mimeType` is silently skipped on Linux: `MimeType=` needs a real MIME type,
and there's no extension-only fallback the way macOS has one. If you want a file association to
register on both platforms, always set `mimeType`.

Launches routed through either mechanism arrive in your React tree via
[`app.onOpenFile`/`app.onOpenUrl`](/native-platform/system-capabilities/#app-level-events). Register
those handlers early, since events fired before the app's first render aren't buffered.

## Update manifest schema

A manifest is a small JSON document, produced by `tools/manifest.ts` (`buildAndSignManifest`) and
consumed by `src/core/update.zig`'s `parseManifest`:

```json
{
  "app_id": "com.nativedesktop.gallery",
  "version": "0.9.0",
  "from": null,
  "full_url": "http://127.0.0.1:0/gallery-0.9.0-linux.tar.zst",
  "full_sig_b64": "<base64 algo‖key_id‖signature blob>"
}
```

- `app_id`: reverse-DNS app identifier.
- `version`: the version this manifest describes.
- `from`: optional, the version a delta artifact would apply against. `null` for a full-only
  manifest.
- `full_url`: where to fetch the full compressed archive.
- `full_sig_b64`: the second base64 line of the archive's `.minisig` file
  (`algorithm[2] ‖ key_id[8] ‖ signature[64]`), passed through verbatim for the Zig verifier to
  decode.
- `delta`: reserved and not populated. The schema leaves room for a future array of delta artifacts,
  so adding delta support later extends the manifest shape instead of breaking it.

## Full-archive updates ship; deltas are deferred

The update payload produced and verified today is a full compressed archive: `.tar.zst` on
Linux (via the devshell's `zstd` CLI) and `.tar.gz` on macOS (via system `tar`), each signed with
minisign. bsdiff/zstd delta-chaining from a previous version is deferred; it would let
an update fetch only the bytes that changed, but it isn't needed to prove the
verify-download-swap flow end to end.

## Signature verification is non-disableable

The minisign/Ed25519 verifier lives in `src/core/update.zig` (`verifyMinisign(pubkey, message,
sig_blob) -> bool`), a pure function with zero I/O. No flag, environment variable, or
manifest field skips verification: every consumer of an update artifact calls the verifier
unconditionally before staging anything. A tampered archive or manifest is rejected; there is no
code path that stages an unverified artifact.

Minisign format, for reference: a `.minisig` file is two base64 lines (each preceded by a comment
line). The first blob decodes to `signature_algorithm[2] ‖ key_id[8] ‖ signature[64]`. Algorithm tag
`Ed` signs the raw message; `ED` (used here, for archives) signs the Blake2b-512 prehash of the
message. The public key file decodes to `signature_algorithm[2] ‖ key_id[8] ‖ public_key[32]`, and
its `key_id` must match the signature's.

## Zero-network update flow

The update flow is tested headlessly against a local Bun HTTP server; no task depends on
`github.com`, `ziglang.org`, or any other remote host at test time. `tools/update-server.ts` serves
a manifest + archive from a directory over `http://127.0.0.1:<ephemeral-port>`; a drive script
fetches both, invokes the Zig verifier, and asserts that a tampered artifact is rejected while a
valid one is accepted and staged atomically.
