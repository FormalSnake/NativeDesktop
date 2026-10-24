+++
title = "Packaging"
description = "Build installers for macOS, Linux, and Windows with nd package, and ship signed delta updates from a static feed with nd release."
+++

`nd package` turns an app into an installer. `nd release` packs it again with deltas, signs the
update feed, and uploads it. Both read `nativedesktop.toml` (see
[settings and config](settings-and-config.md#app-manifest)), and the name and version come from
`Cargo.toml`. [Velopack](https://velopack.io) builds the installers and updates, and nd puts a
signature check in front of its feed.

<!-- docs:example notes -->

## Package an app

```sh
nd package            # this OS
nd package mac        # or linux, windows
```

The command prints the paths of the bundle, the installer, and the Velopack output directory
under `target/nd/package/<os>/`. Flags:

- `--channel`: one of `updates.channels`. The first one is the default.
- `--notes <file.md>`: release notes for "what's new".
- `--binary <path>` and `--runtime <rid>`: pack a binary built elsewhere.
- `--dir`: the app directory.

Packaging needs Velopack's `vpk`. On first use, `nd` installs the pinned version into the user
cache with `dotnet tool install`, so the .NET SDK has to be on the machine. Or set `ND_VPK` to an
existing `vpk`.

| OS | Output | Runs on |
| --- | --- | --- |
| macOS | `.app`, a Velopack `.pkg`, and a `.dmg` | macOS only (codesign, iconutil, hdiutil) |
| Linux | a Velopack AppImage | Linux, or any host with `--binary` |
| Windows | a branded `Setup.exe`, and an MSI if you ask for one | Windows, or any host with `--binary` |

The Linux and Windows packers have unit tests but haven't run on their OS yet. The macOS path has
a full end-to-end gate (`gate_m10`).

## Icons

`app.icon` is a square PNG, at least 512 px. 1024 px is recommended. From it, the packager makes
the `.icns` for macOS, sized PNGs for Linux, and the `.ico` for Windows. It rejects Icon Composer
`.icon` bundles for now, so point `app.icon` at a PNG.

## macOS

The bundle gets an `Info.plist` with the document types and URL schemes from `[app]`. Without a
`signing_identity`, the app is signed ad hoc, which runs on the machine that built it. With
one, Velopack:

1. Signs the bundle inside out, with hardened-runtime entitlements from `[macos] entitlements`
   (`network.client` becomes `com.apple.security.network.client`).
2. Notarizes it through the `notary_profile` that you stored with
   `xcrun notarytool store-credentials`.
3. Builds the `.pkg` with `installer_identity`.

The `.dmg` is signed, notarized, and stapled the same way. `notarize = true` needs both the
profile and the identity, and the manifest fails validation without them.

Chromium for `engine = "cef"` is bundled on macOS and Linux (see
[the Linux webview section](webview.md#linux)). The Windows packer refuses a CEF app.

<!-- docs:snippet examples/notes/nativedesktop.toml#platforms -->

## Linux

The packager stages an AppDir with the binary, a `.desktop` file that carries `MimeType=` and the
URL schemes, the shared-mime XML, and icons. Velopack turns it into an AppImage. A bare AppImage
registers nothing with the desktop. An integrator such as AppImageLauncher installs the `.desktop`
file and the MIME types. `formats` accepts only `appimage` for now. `deb` and `flatpak` are in the
schema, but the packager fails on them. An app with web views also gets the Chromium runtime
beside its executable; see [webview](webview.md#linux) for the system libraries it needs.

## Windows

`Setup.exe` takes its name, icon, splash, publisher, accent, and shortcuts from `[windows]`, and
`installer.msi = true` adds an MSI. `signing = "azure"` signs through Azure Artifact Signing,
which runs only on a Windows host. A cross pack from macOS or Linux signs with
`ND_WINDOWS_SIGN_TEMPLATE`, a command that contains `{{file}}`. Without it, the pack fails
instead of shipping unsigned by accident. `Setup.exe` doesn't register file associations or URL
schemes yet. `files` lists paths, relative to `nativedesktop.toml`, that are copied beside the
executable, such as a helper `.exe` the app starts.

## Releases and updates

An update is a directory of static files with relative names: `releases.<os>-<channel>.json`, its
`.sig`, the full and delta packages, and the release notes. Any static host can serve it. Velopack
channels include the OS, such as `mac-stable`, `linux-beta`, and `win-stable`, so every OS shares
one directory (`target/nd/releases`) without overwriting the others.

The app carries the feed URL and an ed25519 public key. `nd release keygen` makes the key pair,
writes the private key to `~/.config/nd/keys/<app id>.key` (mode 0600), and prints the line for
`[updates] public_key`. The signature is base64 ed25519 over `nd-feed-v1`, the channel, and the
feed bytes, so a host can't swap one channel's feed for another. The client checks the signature
before Velopack reads the feed, because Velopack on its own trusts whatever hashes the feed lists.
The client refuses a tampered feed before it downloads anything.

<!-- docs:snippet examples/notes/nativedesktop.toml#updates -->

```sh
nd release keygen
nd release --channel beta --notes notes.md
```

`nd release` packs against the previous release so that Velopack can build deltas, signs the
feed, and then publishes. On a fresh checkout, it first downloads the last release from the live
feed. If that fails, it says so and ships without deltas. The signing key comes from `--key`,
then `ND_UPDATE_SIGNING_KEY`, then the keygen path. `nd release` refuses a key that doesn't match
`updates.public_key`. `--no-publish` writes and signs the directory without uploading it.

The first update after a `.pkg` install is a full download, because the installer clears
Velopack's package cache and leaves nothing to diff against. Later updates are deltas.

### Publishers

`[updates.publish]` chooses where the directory goes. None of the publishers talks to a service
run by the nd project.

- `kind = "s3"`: R2, S3, or MinIO. Set `bucket`, `prefix`, `region` (R2 takes `auto`), and either
  `endpoint` or `endpoint_env` to keep an account id out of the file. Credentials come from `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`.
- `kind = "github"`: one rolling release tagged `updates`, uploaded with `gh`.
- `kind = "rsync"`: `target` is a local path or `host:path`.
- `kind = "dir"`: a local directory. `file://` update tests read from this one.

Packages upload first, and the feed with its signature uploads last, so a client never sees a
feed that lists a package that isn't there yet.

### Self-hosted static feed

Without a publisher, `nd release --no-publish` leaves a directory that you can host anywhere: an
R2 bucket behind a domain, nginx on a VPS, or GitHub Pages. Set `updates.feed` to its URL. To
test, serve it locally:

```sh
nd release serve --port 8790
```

It serves `target/nd/releases` (or `--root`) on `127.0.0.1` and logs one line per request. The
public key is in the app, so moving to another host never weakens verification.

### In the app

A packaged app checks for updates without any code of its own. `Application::run` calls the
`nd::update` startup, waits 10 seconds (`ND_UPDATE_DELAY_MS`), and then checks every six hours
until it downloads an update. The update applies when the app quits. `Updater` also has
`apply_and_restart` for a "Restart now" button.

After an update, the app keeps the release notes once. `nd::whats_new(&manifest)` returns them as
Markdown and HTML until you call `nd::mark_whats_new_seen(&manifest)`, so the app shows a "what's
new" view only one time. No example app draws that view yet.

### Where updates are off

`nd::update::Unavailable` says why a process doesn't update: there's no `[updates]` table, the
build set `ND_NO_UPDATES=1`, the binary runs from `/nix/store`, it's the web, or Velopack didn't
install the binary (`cargo run`, tests). `mkApp` and the generated `fspec.star` set
`ND_NO_UPDATES=1`, because Nix or fspec owns the files there. The web build compiles the updater out. See [web](web.md).
