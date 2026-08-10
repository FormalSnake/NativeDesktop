# Packaging + updates (M9)

This document covers `nd package <platform>` (packaging the gallery example
into a distributable), the update manifest format, and how the auto-update
flow verifies what it downloads. It also states explicitly what M9 does not
do; every deferral below is a deliberate scoping call.

`nd package` is a documented convention, not a shipped binary (the same
convention as `nd codegen` ≡ `bun tools/codegen.ts`; see
`template/README.md`). There is no `bin/nd` dispatcher; `nd package <platform>`
means `bun tools/package.ts <platform>`.

## Commands

```bash
bun tools/package.ts linux   # AppImage (or squashfs fallback) + signed .tar.zst update
bun tools/package.ts mac     # Gallery.app (deep-signed) + signed .tar.gz update
```

`ND_APP_VERSION` (default `0.9.0`) sets the packaged version and the version
recorded in the update manifest.

Windows packaging (signed NSIS installer + winget manifest) lands with M7,
which is not yet implemented.
`tools/package.ts` exits with an error and a usage message for any platform
argument other than `linux`/`mac`.

## Linux: AppImage + Flatpak (M9-D4)

`tools/package-linux.ts` assembles a self-contained AppDir under
`dist/linux/AppDir` from `packaging/AppDir.template/` (`.desktop`, icon,
`AppRun`), the built `nd-hello` binary, the Bun runtime, and the app sources
(examples/packages/runtime, re-`bun install --production`'d inside the
AppDir; see "Bun workspace bundling" below). It then packs the AppDir into
an AppImage with `appimagetool`, or with `mksquashfs` when `appimagetool`
isn't on `PATH` (it currently is not in the nix devshell, so CI and local
runs exercise the squashfs fallback path).

A committed Flatpak manifest also exists at
`packaging/flatpak/com.nativedesktop.gallery.yml` (GNOME 50 runtime,
`nd-hello` command, wayland/fallback-x11/dri finish-args only; no portal
permissions, since in-process automation needs none per spec §11).
The M9 gate only lint-validates this manifest
(`flatpak-builder --show-manifest packaging/flatpak/com.nativedesktop.gallery.yml`);
it does not run a full `flatpak-builder` build. `flatpak-builder` is
fragile inside a nix sandbox or stock CI (portal + runtime + bubblewrap
requirements), so the full build is deferred to a real GNOME runner and is
not part of `scripts/headless-m9.sh` or `.github/workflows/package.yml`.

## macOS: .app + codesign + notarization (M9-D3)

`tools/package-mac.ts` assembles `Gallery.app` around
`swift/.build/release/NDShell`, the bundled Bun runtime, and the app
sources, then deep-signs inside-out (nested Mach-O first, `bun` and
`NDShell`, then the `.app` itself) with the hardened runtime
(`--options runtime`) and a `com.apple.security.cs.allow-jit` entitlement
(`packaging/macos/entitlements.plist`; JSC-under-Bun needs it on Apple
Silicon, spec §11).

**Signing identity resolution:** if `APPLE_SIGN_IDENTITY` is set, it's used
as the `codesign -s` identity (Developer ID / Team-ID-backed signing);
otherwise packaging falls back to ad-hoc signing (`codesign -s -`).

**Ad-hoc + hardened runtime:** `--options runtime` is
accepted by `codesign` alongside an ad-hoc (`-s -`) signature, and the
entitlements are embedded, but the OS only enforces hardened-runtime
protections (library validation, restricted entitlements, etc.) for
signatures backed by a valid Team ID. An ad-hoc-signed `.app` still launches
locally (and over the ssh-driven session used in this repo's Mac legs), and
`codesign --verify` passes; that's all the M9 gate asserts
(`ND_PACKAGE_APP_SIGNED ... identity=adhoc`, `MAC_M9_CODESIGN_OK`). It is
not the same guarantee a Team-ID signature gives; treat ad-hoc as "runs
here," not "safe to distribute."

**Notarization is gated on secrets.**
`xcrun notarytool submit` + `xcrun stapler staple` run only when all
three of `APPLE_ID`, `APPLE_TEAM_ID`, and `APPLE_APP_PASSWORD` are present
in the environment. No such credentials exist in this repo's dev/CI
environment, so packaging always takes the skip path and prints
`ND_PACKAGE_NOTARIZE_SKIPPED reason=no-credentials`. This is the expected,
asserted behavior of `scripts/mac/mac-m9.sh` and the `macos-package` CI
job; it is not a bug and does not fail the gate.

**Mac machine prerequisite:** the Mac dev loop signs update archives with
`minisign`, which is not part of Xcode or the system toolchain. Install it
with `brew install minisign` on the Mac (the CI job installs it as a step;
a developer's machine needs it pre-installed, same as Zig/Bun).

## Update manifest schema

A manifest is a small JSON document, produced by `tools/manifest.ts`
(`buildAndSignManifest`) and consumed by `src/core/update.zig`'s
`parseManifest`:

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
- `from`: optional, the version a delta artifact (see below) would apply
  against. `null` for a full-only manifest.
- `full_url`: where to fetch the full compressed archive.
- `full_sig_b64`: the second base64 line of the archive's `.minisig` file
  (`algorithm[2] ‖ key_id[8] ‖ signature[64]`), passed through verbatim for
  the Zig verifier to decode.
- `delta`: reserved and not populated. The schema leaves room for a
  future array of delta artifacts (each naming a `from` version, a URL, and
  its own signature) so that adding delta support later is additive to the
  manifest shape rather than a breaking change.

## Full-archive updates ship; deltas are deferred (M9-D2)

The only update payload M9 produces and verifies is a full compressed
archive: `.tar.zst` on Linux (via the devshell's `zstd` CLI) and `.tar.gz`
on macOS (via system `tar`), each signed with minisign. zig-bsdiff +
zstd delta-chaining from a previous version is deferred: it
would let an update fetch only the bytes that changed instead of a full
archive, but it pulls in a bsdiff dependency and chaining logic that isn't
needed to prove the verify-download-swap flow end to end. The manifest's
reserved `delta` field (see above) is the only concession M9 makes toward
that future work.

## Signature verification is non-disableable

The minisign/Ed25519 verifier lives in `src/core/update.zig`
(`verifyMinisign(pubkey, message, sig_blob) -> bool`), a pure function with
zero I/O; it only ever operates on byte slices handed to it. No flag,
environment variable, or manifest field skips verification:
every consumer of an update artifact (the `zig build update-verify` CLI,
and `scripts/m9-drive.ts` which shells out to it) calls the verifier
unconditionally before staging anything. A tampered archive or manifest is
rejected; there is no code path that stages an unverified artifact.

Minisign format, for reference: a `.minisig` file is two base64 lines (each
preceded by a comment line). The first blob decodes to
`signature_algorithm[2] ‖ key_id[8] ‖ signature[64]`. Algorithm tag `Ed`
signs the raw message; `ED` (used here, for archives, which are large)
signs the Blake2b-512 prehash of the message. The public key file decodes to
`signature_algorithm[2] ‖ key_id[8] ‖ public_key[32]`, and its `key_id` must
match the signature's.

## Zero-network update flow (M9-D5)

The update flow is tested headlessly against a local Bun HTTP server;
no task depends on `github.com`, `ziglang.org`, or any other remote host at
test time. `tools/update-server.ts` serves a manifest + archive from a
directory over `http://127.0.0.1:<ephemeral-port>`; `scripts/m9-drive.ts`
fetches both, invokes the Zig verifier, and asserts that a tampered
artifact is rejected while a valid one is accepted and staged atomically.

Cross-references: `scripts/headless-m9.sh` (Linux: package, launch the
packaged AppDir headlessly under weston, then run the update flow) and
`scripts/mac/mac-m9.sh` (macOS: package, ad-hoc-sign, launch the packaged
`.app` in-session, then run the update flow). Both scripts clean `dist/`
first; the packers are not idempotent against a leftover output tree from
a prior run.

## CI (`.github/workflows/package.yml`)

- `linux-package` (blocking): installs nix, builds, runs
  `./scripts/headless-m9.sh`, then lint-validates the Flatpak manifest.
- `macos-package` (stretch, non-blocking via `continue-on-error: true`,
  the same convention as `.github/workflows/mac.yml`): installs Zig + Bun +
  minisign on a stock `macos-latest` runner, builds the Swift shell,
  packages and ad-hoc-signs `Gallery.app`, verifies the signature, and
  uploads the `.app` as a build artifact. A red mac job never blocks a
  merge. As with `mac.yml`, the job has been validated as lint-clean YAML
  locally; it has not yet been observed running on a real runner.
