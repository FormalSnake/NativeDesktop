---
title: Packaging
description: nd package, the update manifest format, and how the auto-update flow verifies what it downloads.
---

`nd package <platform>` is a documented convention, not a shipped subcommand — same convention as
`nd codegen` (≡ `bun tools/codegen.ts`). A `bin/nd` dispatcher does exist now (`packages/nd`, see
[Project Layout](/get-started/project-layout/)), but it only implements `nd dev`/`nd build` for
running and compiling an *app*; `nd package <platform>` still means `bun tools/package.ts
<platform>` directly.

## Commands

```bash
bun tools/package.ts linux   # AppImage (or squashfs fallback) + signed .tar.zst update
bun tools/package.ts mac     # Gallery.app (deep-signed) + signed .tar.gz update
```

`ND_APP_VERSION` (default `0.9.0`) sets the packaged version and the version recorded in the update
manifest.

**Windows packaging (signed NSIS installer + winget manifest) lands with the Windows backend, which
is not yet implemented** — `tools/package.ts` exits with an error and a usage message for any
platform argument other than `linux`/`mac`. See [Platform Support](/native-platform/platform-support/).

## Linux: AppImage + Flatpak

`tools/package-linux.ts` assembles a self-contained AppDir under `dist/linux/AppDir` from
`packaging/AppDir.template/` (`.desktop`, icon, `AppRun`), the built `nd-hello` binary, the Bun
runtime, and the app sources (re-`bun install --production`'d inside the AppDir). It packs the
AppDir into an AppImage with `appimagetool`, falling back to `mksquashfs` if `appimagetool` isn't on
`PATH` — it currently is not in the nix devshell, so CI and local runs exercise the squashfs fallback
path.

A committed Flatpak manifest also exists at `packaging/flatpak/com.nativedesktop.gallery.yml` (GNOME
50 runtime, `nd-hello` command, wayland/fallback-x11/dri finish-args only — no portal permissions,
since in-process automation needs none). **The CI gate only lint-validates this manifest**
(`flatpak-builder --show-manifest packaging/flatpak/com.nativedesktop.gallery.yml`) — it does not run
a full `flatpak-builder` build, since that's fragile inside a nix sandbox or stock CI (portal +
runtime + bubblewrap requirements). The full build is deferred to a real GNOME runner.

## macOS: `.app` + codesign + notarization

`tools/package-mac.ts` assembles `Gallery.app` around `swift/.build/release/NDShell`, the bundled
Bun runtime, and the app sources, then deep-signs inside-out (nested Mach-O first — `bun`,
`NDShell` — then the `.app` itself) with the hardened runtime (`--options runtime`) and a
`com.apple.security.cs.allow-jit` entitlement (JSC-under-Bun needs it on Apple Silicon).

**Signing identity:** if `APPLE_SIGN_IDENTITY` is set, it's used as the `codesign -s` identity;
otherwise packaging falls back to **ad-hoc** (`codesign -s -`).

**Ad-hoc + hardened runtime, the honest truth:** `--options runtime` is accepted alongside an ad-hoc
signature, and the entitlements are embedded — but the OS only *enforces* hardened-runtime
protections for signatures backed by a valid Team ID. An ad-hoc-signed `.app` still launches locally
and `codesign --verify` passes — that's all the packaging gate asserts. It is **not** the same
guarantee a Team-ID signature gives; treat ad-hoc as "runs here," not "safe to distribute."

**Notarization is gated on secrets, not disabled by policy.** `xcrun notarytool submit` +
`xcrun stapler staple` run only when `APPLE_ID`, `APPLE_TEAM_ID`, and `APPLE_APP_PASSWORD` are all
present in the environment. No such credentials exist in this repo's dev/CI environment, so
packaging always takes the skip path and prints `ND_PACKAGE_NOTARIZE_SKIPPED reason=no-credentials`
— this is expected, not a bug.

**Mac prerequisite:** the Mac packaging flow signs update archives with `minisign`, which is not
part of Xcode or the system toolchain — install it with `brew install minisign`.

## File associations and URL schemes

`nd package` stamps an app's identity — a bundle id override, document types, and custom URL
schemes — into the platform packaging outputs, driven by the `app` field of
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

- **`id`** overrides `CFBundleIdentifier` on macOS. There's no Linux equivalent — the `.desktop`
  file's identity comes from its filename.
- **`fileAssociations`** — each entry is one document type: `ext` (no dot), an optional
  human-readable `name`, an optional `mimeType`, and `role` (`"editor"` or `"viewer"`, default
  `"editor"`).
- **`urlSchemes`** — each entry is one custom scheme (`{ scheme, name? }`), e.g. `myapp://...`.

On macOS, `tools/app-identity.ts` injects `CFBundleDocumentTypes` (one `<dict>` per file
association) and `CFBundleURLTypes` (one per URL scheme) into `Info.plist`, alongside the
`CFBundleIdentifier` override when `id` is set. `fileAssociations` work here even without a
`mimeType` — macOS keys document types off the extension and `UTType`, not a MIME type.

On Linux, the same config extends the AppDir's `.desktop` entry: a `MimeType=` line built from every
file association's `mimeType` plus `x-scheme-handler/<scheme>` for each URL scheme, and an `Exec=...
%U` argument so the launched app receives the opened path/URL. A `usr/share/mime/packages/<app>.xml`
shared-mime-info file is also generated for any file association that declares a `mimeType`. **A file
association without a `mimeType` is silently skipped on Linux** — `MimeType=` needs a real MIME type,
there's no extension-only fallback the way macOS has one. If you want a file association to register
on both platforms, always set `mimeType`.

Launches routed through either mechanism arrive in your React tree via
[`app.onOpenFile`/`app.onOpenUrl`](/native-platform/system-capabilities/#app-level-events) — register
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

- `app_id` — reverse-DNS app identifier.
- `version` — the version this manifest describes.
- `from` — optional; the version a delta artifact would apply against. `null` for a full-only
  manifest.
- `full_url` — where to fetch the full compressed archive.
- `full_sig_b64` — the second base64 line of the archive's `.minisig` file
  (`algorithm[2] ‖ key_id[8] ‖ signature[64]`), passed through verbatim for the Zig verifier to
  decode.
- `delta` — **reserved, not populated.** The schema leaves room for a future array of delta
  artifacts so adding delta support later is additive to the manifest shape, not a breaking flag day.

## Full-archive updates ship; deltas are deferred

The only update payload produced and verified today is a **full compressed archive**: `.tar.zst` on
Linux (via the devshell's `zstd` CLI) and `.tar.gz` on macOS (via system `tar`), each signed with
minisign. bsdiff/zstd delta-chaining from a previous version is explicitly deferred — it would let
an update fetch only the bytes that changed, but it isn't needed to prove the
verify-download-swap flow end to end.

## Signature verification is non-disableable

The minisign/Ed25519 verifier lives in `src/core/update.zig` (`verifyMinisign(pubkey, message,
sig_blob) -> bool`), a pure function with zero I/O. There is **no flag, environment variable, or
manifest field that skips verification** — every consumer of an update artifact calls the verifier
unconditionally before staging anything. A tampered archive or manifest is rejected; there is no
code path that stages an unverified artifact.

Minisign format, for reference: a `.minisig` file is two base64 lines (each preceded by a comment
line). The first blob decodes to `signature_algorithm[2] ‖ key_id[8] ‖ signature[64]`. Algorithm tag
`Ed` signs the raw message; `ED` (used here, for archives) signs the Blake2b-512 prehash of the
message. The public key file decodes to `signature_algorithm[2] ‖ key_id[8] ‖ public_key[32]`, and
its `key_id` must match the signature's.

## Zero-network update flow

The update flow is tested headlessly against a **local** Bun HTTP server — no task depends on
`github.com`, `ziglang.org`, or any other remote host at test time. `tools/update-server.ts` serves
a manifest + archive from a directory over `http://127.0.0.1:<ephemeral-port>`; a drive script
fetches both, invokes the Zig verifier, and asserts that a tampered artifact is rejected while a
valid one is accepted and staged atomically.
