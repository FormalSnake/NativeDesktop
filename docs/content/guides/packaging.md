+++
title = "Packaging"
description = "Build installers for macOS, Linux and Windows with nd package, and ship signed delta updates from a static feed with nd release."
+++

`nd package` turns an app into an installer. `nd release` packs it again with deltas, signs the
update feed and uploads it. Both read `nativedesktop.toml` (see
[settings and config](settings-and-config.md#app-manifest)); name and version come from
`Cargo.toml`. The installers and updates are built by [Velopack](https://velopack.io), with a
signature check in front of its feed.

<!-- docs:example notes -->

## Package an app

```sh
nd package            # this OS
nd package mac        # or linux, windows
```

It prints the bundle, the installer and the Velopack output directory under
`target/nd/package/<os>/`. Flags: `--channel` (one of `updates.channels`, the first by default),
`--notes <file.md>` (release notes for "what's new"), `--binary <path>` and `--runtime <rid>` to
pack a binary built elsewhere, and `--dir` for the app directory.

Packaging needs Velopack's `vpk`. `nd` installs the pinned version into the user cache with
`dotnet tool install` on first use, so the .NET SDK has to be on the machine, or set `ND_VPK` to
an existing `vpk`.

| OS | Output | Runs on |
| --- | --- | --- |
| macOS | `.app`, a Velopack `.pkg` and a `.dmg` | macOS only (codesign, iconutil, hdiutil) |
| Linux | a Velopack AppImage | Linux, or any host with `--binary` |
| Windows | a branded `Setup.exe`, an MSI when asked | Windows, or any host with `--binary` |

The Linux and Windows packers have unit tests and no run on their OS yet. The macOS path has a
full end-to-end gate (`gate_m10`).

## Icons

`app.icon` is a square PNG, 512 px at least and 1024 recommended. The packager makes the `.icns`
for macOS, sized PNGs for Linux and the `.ico` for Windows from it. Icon Composer `.icon` bundles
are rejected for now, so point `app.icon` at a PNG.

## macOS

The bundle gets an `Info.plist` with the document types and URL schemes from `[app]`. Without a
`signing_identity` the app is signed ad hoc, which runs on the machine that built it. With one,
Velopack signs inside out with hardened-runtime entitlements taken from `[macos] entitlements`
(`network.client` becomes `com.apple.security.network.client`), notarizes through the
`notary_profile` you stored with `xcrun notarytool store-credentials`, and builds the `.pkg` with
`installer_identity`. The `.dmg` is signed, notarized and stapled the same way. `notarize = true`
needs both the profile and the identity, and the manifest fails validation without them.

Chromium for `engine = "cef"` is bundled on macOS only; the Linux and Windows packers refuse a CEF
app.

<!-- docs:snippet examples/notes/nativedesktop.toml#platforms -->

## Linux

A staged AppDir with the binary, a `.desktop` file carrying `MimeType=` and the URL schemes, the
shared-mime XML and icons, which Velopack turns into an AppImage. A bare AppImage registers
nothing with the desktop; an integrator such as AppImageLauncher installs the `.desktop` and the
MIME types. `formats` accepts only `appimage` for now; `deb` and `flatpak` are in the schema and
fail the packager.

## Windows

`Setup.exe` takes its name, icon, splash, publisher, accent and shortcuts from `[windows]`, and
`installer.msi = true` adds an MSI. `signing = "azure"` signs through Azure Artifact Signing, which
runs only on a Windows host. A cross pack from macOS or Linux signs with
`ND_WINDOWS_SIGN_TEMPLATE`, a command containing `{{file}}`, and otherwise refuses instead of
shipping unsigned by accident. `Setup.exe` does not register file associations or URL schemes yet.

## Releases and updates

An update is a directory of static files with relative names: `releases.<os>-<channel>.json`, its
`.sig`, the full and delta packages and the release notes. Any static host can serve it. Velopack
channels are `mac-stable`, `linux-beta`, `win-stable`, so every OS shares one directory
(`target/nd/releases`) without overwriting each other.

The app carries the feed URL and an ed25519 public key. `nd release keygen` makes the key pair,
writes the private key to `~/.config/nd/keys/<app id>.key` (mode 0600) and prints the line for
`[updates] public_key`. The signature is base64 ed25519 over `nd-feed-v1`, the channel and the
feed bytes, so a host cannot swap one channel's feed for another. The client checks the signature
before Velopack reads the feed, because Velopack alone trusts whatever hashes the feed lists. A
tampered feed is refused before any download.

<!-- docs:snippet examples/notes/nativedesktop.toml#updates -->

```sh
nd release keygen
nd release --channel beta --notes notes.md
```

`nd release` packs against the previous release so Velopack can build deltas, signs the feed, then
publishes. On a fresh checkout it downloads the last release from the live feed first; if that
fails it says so and ships without deltas. The signing key comes from `--key`, then
`ND_UPDATE_SIGNING_KEY`, then the keygen path. It refuses a key that does not match
`updates.public_key`. `--no-publish` writes and signs the directory without uploading it.

The first update after a `.pkg` install is a full download, because the installer clears
Velopack's package cache and there is nothing to diff against. Later updates are deltas.

### Publishers

`[updates.publish]` picks where the directory goes. None of them talks to a service of ours.

- `kind = "s3"`: R2, S3, MinIO. `bucket`, `prefix`, `region` (R2 takes `auto`), and `endpoint` or
  `endpoint_env` to keep an account id out of the file. Credentials come from `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`.
- `kind = "github"`: one rolling release tagged `updates`, uploaded with `gh`.
- `kind = "rsync"`: `target` is a local path or `host:path`.
- `kind = "dir"`: a local directory, also what `file://` update tests read.

Packages upload first and the feed with its signature last, so a client never sees a feed that
lists a package that is not there yet.

### Self-hosted static feed

Without a publisher, `nd release --no-publish` leaves a directory you can put anywhere: an R2
bucket behind a domain, nginx on a VPS, GitHub Pages. Set `updates.feed` to its URL. For testing,
serve it locally:

```sh
nd release serve --port 8790
```

It serves `target/nd/releases` (or `--root`) on `127.0.0.1` with one log line per request. The
public key is in the app, so moving hosts never weakens verification.

### In the app

A packaged app checks without any code of its own: `Application::run` calls `nd::update` startup,
waits 10 seconds (`ND_UPDATE_DELAY_MS`), then checks every six hours until one is downloaded,
which applies on quit. `Updater` also has `apply_and_restart` for a "restart now" button.

After an update the release notes are kept once. `nd::whats_new(&manifest)` returns them as
Markdown and HTML until `nd::mark_whats_new_seen(&manifest)` is called, so the app shows a "what's
new" view a single time. No example app draws that view yet.

### Where updates are off

`nd::update::Unavailable` says why a process does not update: no `[updates]` table, a build with
`ND_NO_UPDATES=1`, a binary running from `/nix/store`, the web, or a binary that was not installed
by Velopack (`cargo run`, tests). `mkApp` sets `ND_NO_UPDATES=1` for Nix builds, and Nix owns the
files there. The web build compiles the updater out. See [web](web.md).
