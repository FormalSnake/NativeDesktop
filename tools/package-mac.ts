#!/usr/bin/env bun
// tools/package-mac.ts — `nd package mac` ≡ bun tools/package.ts mac.
// Assembles Gallery.app around swift/.build/release/NDShell + the Zig core
// (already linked into NDShell) + Bun + the gallery, deep-signs the nested
// binaries inside-out (ad-hoc by default; Developer-ID if APPLE_SIGN_IDENTITY
// is set) with the hardened runtime + allow-jit entitlements, and gates
// notarization on Apple credential presence.
import { $ } from "bun";
import { mkdirSync, cpSync, chmodSync, readFileSync, writeFileSync } from "node:fs";
import { buildAndSignManifest, ensureEphemeralKey } from "./manifest.ts";
import { loadConfig } from "../packages/nd/src/config.ts";
import { injectInfoPlist } from "./app-identity.ts";

const VERSION = process.env.ND_APP_VERSION ?? "0.9.0";
const APP_ID = "com.nativedesktop.gallery";

export async function packageMac() {
  const dist = "dist/mac", updDir = "dist/update";
  const app = `${dist}/Gallery.app`, c = `${app}/Contents`;
  mkdirSync(`${c}/MacOS`, { recursive: true });
  mkdirSync(`${c}/Resources`, { recursive: true });
  mkdirSync(`${c}/Frameworks`, { recursive: true });
  mkdirSync(updDir, { recursive: true });

  cpSync("packaging/macos/Info.plist", `${c}/Info.plist`);
  const { app: appIdentity } = await loadConfig("examples/gallery");
  if (appIdentity) writeFileSync(`${c}/Info.plist`, injectInfoPlist(readFileSync(`${c}/Info.plist`, "utf8"), appIdentity));
  cpSync("swift/.build/release/NDShell", `${c}/MacOS/NDShell`);
  chmodSync(`${c}/MacOS/NDShell`, 0o755);
  const bunPath = Bun.which("bun");
  if (!bunPath) throw new Error("bun not found on PATH");
  // dereference: true — `bun` on PATH is frequently a symlink into a
  // read-only nix store; copying the link itself would leave a dangling/
  // unwritable entry once relocated into the .app (chmod EPERM on the nix
  // store target), so resolve it to the real file.
  cpSync(bunPath, `${c}/MacOS/bun`, { dereference: true }); chmodSync(`${c}/MacOS/bun`, 0o755);
  // dereference: true — examples/packages node_modules are Bun-workspace
  // symlinks into the root node_modules/.bun store; a raw symlink copy would
  // dangle once relocated into the .app, so resolve them to real files.
  cpSync("examples", `${c}/Resources/examples`, { recursive: true, dereference: true });
  cpSync("packages", `${c}/Resources/packages`, { recursive: true, dereference: true });
  // packages/react's src imports "../../../runtime/ndp.ts" — bundle runtime/
  // at the matching relative depth so that resolves inside the .app too.
  cpSync("runtime", `${c}/Resources/runtime`, { recursive: true, dereference: true });

  // Signing identity: Developer-ID if set, else ad-hoc.
  const identity = process.env.APPLE_SIGN_IDENTITY ?? "-";
  const ent = "packaging/macos/entitlements.plist";
  // Deep-sign inside-out: nested Mach-O first, the .app last.
  for (const nested of [`${c}/MacOS/bun`, `${c}/MacOS/NDShell`]) {
    await $`codesign --force --options runtime --entitlements ${ent} --sign ${identity} ${nested}`;
  }
  await $`codesign --force --deep --options runtime --entitlements ${ent} --sign ${identity} ${app}`;
  await $`codesign --verify --strict ${app}`;
  console.error(`ND_PACKAGE_APP_SIGNED ${app} identity=${identity === "-" ? "adhoc" : "developer-id"}`);

  // Notarization gated on credentials.
  const { APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD } = process.env;
  if (APPLE_ID && APPLE_TEAM_ID && APPLE_APP_PASSWORD) {
    const zip = `${dist}/Gallery.zip`;
    await $`ditto -c -k --keepParent ${app} ${zip}`;
    await $`xcrun notarytool submit ${zip} --apple-id ${APPLE_ID} --team-id ${APPLE_TEAM_ID} --password ${APPLE_APP_PASSWORD} --wait`;
    await $`xcrun stapler staple ${app}`;
    console.error(`ND_PACKAGE_NOTARIZE_OK ${app}`);
  } else {
    console.error(`ND_PACKAGE_NOTARIZE_SKIPPED reason=no-credentials`);
  }

  // Full-archive update payload (.tar.gz on mac) + signed manifest.
  const archive = `${updDir}/gallery-${VERSION}-mac.tar.gz`;
  await $`tar -C ${dist} -czf ${archive} Gallery.app`;
  const { sec, pub } = await ensureEphemeralKey(updDir);
  const { manifestPath } = await buildAndSignManifest({
    appId: APP_ID, version: VERSION, from: null,
    archivePath: archive, url: `http://127.0.0.1:0/${archive.split("/").pop()}`,
    secKey: sec, pubKey: pub,
  });
  console.error(`ND_PACKAGE_MANIFEST ${manifestPath} pub=${pub}`);
}

if (import.meta.main) {
  await packageMac();
}
