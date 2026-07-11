#!/usr/bin/env bun
// tools/package-linux.ts — `nd package linux`: assembles an AppDir for the
// gallery example, packs it into an AppImage (falling back to a squashfs
// image if appimagetool isn't available), then produces a signed full-archive
// update payload (M9-D2/D4).
import { $ } from "bun";
import { mkdirSync, cpSync, chmodSync } from "node:fs";
import { buildAndSignManifest, ensureEphemeralKey } from "./manifest.ts";

const VERSION = process.env.ND_APP_VERSION ?? "0.9.0";
const APP_ID = "com.nativedesktop.gallery";

export async function packageLinux() {
  const dist = "dist/linux", updDir = "dist/update";
  mkdirSync(dist, { recursive: true });
  mkdirSync(updDir, { recursive: true });
  const appdir = `${dist}/AppDir`;
  cpSync("packaging/AppDir.template", appdir, { recursive: true });
  mkdirSync(`${appdir}/usr/bin`, { recursive: true });
  cpSync("zig-out/bin/nd-hello", `${appdir}/usr/bin/nd-hello`);
  chmodSync(`${appdir}/usr/bin/nd-hello`, 0o755);
  chmodSync(`${appdir}/AppRun`, 0o755);
  // Bundle the app source (the gallery is script-driven, not compiled). The
  // workspace's node_modules are Bun-hoisted symlinks into the root
  // node_modules/.bun store (including transitive deps like `scheduler`
  // pulled in by react-reconciler, which aren't even symlinked at the leaf
  // package level — only Bun's own resolver knows how to find them there).
  // Rather than hand-roll that resolution, reproduce the workspace root
  // (package.json + bun.lock + sources) inside the AppDir and let `bun
  // install` populate a real, self-contained node_modules for it.
  const appRoot = `${appdir}/app`;
  mkdirSync(appRoot, { recursive: true });
  cpSync("package.json", `${appRoot}/package.json`);
  cpSync("bun.lock", `${appRoot}/bun.lock`);
  cpSync("examples", `${appRoot}/examples`, { recursive: true, dereference: true });
  cpSync("packages", `${appRoot}/packages`, { recursive: true, dereference: true });
  // packages/react's src imports "../../../runtime/ndp.ts" — bundle runtime/
  // at the matching relative depth so that resolves inside the AppDir too.
  cpSync("runtime", `${appRoot}/runtime`, { recursive: true, dereference: true });
  await $`bun install --frozen-lockfile --production`.cwd(appRoot).quiet();
  // Bundle the Bun runtime the host spawns.
  const bunPath = Bun.which("bun");
  if (!bunPath) throw new Error("bun not found on PATH");
  cpSync(bunPath, `${appdir}/usr/bin/bun`);
  chmodSync(`${appdir}/usr/bin/bun`, 0o755);

  // AppImage assembly. appimagetool if available; else mksquashfs.
  const appImage = `${dist}/Gallery-${VERSION}.AppImage`;
  await $`appimagetool ${appdir} ${appImage}`.quiet()
    .catch(async () => { await $`mksquashfs ${appdir} ${appImage} -root-owned -noappend`.quiet(); });
  console.error(`ND_PACKAGE_APPIMAGE ${appImage}`);

  // Full-archive update payload (.tar.zst), signed (M9-D2).
  const archive = `${updDir}/gallery-${VERSION}-linux.tar.zst`;
  await $`tar -C ${dist} -cf - AppDir | zstd -q -o ${archive}`.quiet();
  const { sec, pub } = await ensureEphemeralKey(updDir);
  const { manifestPath } = await buildAndSignManifest({
    appId: APP_ID, version: VERSION, from: null,
    archivePath: archive, url: `http://127.0.0.1:0/${archive.split("/").pop()}`,
    secKey: sec, pubKey: pub,
  });
  console.error(`ND_PACKAGE_MANIFEST ${manifestPath} pub=${pub}`);
}

if (import.meta.main) {
  await packageLinux();
}
