// Linux packaging: assembles a self-contained AppDir (generated AppRun +
// .desktop + icons + mime XML, the gtk host binary, bundled Bun, the app
// payload), then packs it into an AppImage with appimagetool, falling back to
// mksquashfs when appimagetool isn't on PATH.
import { $ } from "bun";
import { chmodSync, cpSync, mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { resolveHostBinary } from "@nativedesktop/host";
import type { NativeDesktopConfig } from "../config.ts";
import { installLinuxIcon } from "./icons.ts";
import { buildAppRun, buildDesktopEntry, buildMimeInfoXml, type ResolvedIdentity } from "./identity.ts";
import { assemblePayload } from "./payload.ts";
import { maybePublishUpdate } from "./updates.ts";
import type { PackageOptions, PackageResult } from "./index.ts";

export async function packageLinuxApp(
  appDir: string,
  config: NativeDesktopConfig,
  identity: ResolvedIdentity,
  outDir: string,
  options: PackageOptions,
): Promise<PackageResult> {
  const linux = config.package?.linux;
  const dist = join(outDir, "linux");
  const appdir = join(dist, "AppDir");
  if (linux?.appDirTemplate) {
    cpSync(resolve(appDir, linux.appDirTemplate), appdir, { recursive: true });
  }
  mkdirSync(join(appdir, "usr", "bin"), { recursive: true });

  const payload = await assemblePayload({
    appDir,
    config,
    identity,
    appRoot: join(appdir, "app"),
    entry: options.entry,
    compile: options.compile,
  });

  writeFileSync(join(appdir, "AppRun"), buildAppRun({
    entry: payload.entry,
    cwd: payload.cwd,
    slug: identity.slug,
    appId: identity.id,
    pluginPaths: payload.pluginPaths,
  }));
  chmodSync(join(appdir, "AppRun"), 0o755);
  writeFileSync(join(appdir, `${identity.slug}.desktop`), buildDesktopEntry(identity, linux));
  await installLinuxIcon(identity, appDir, appdir);
  const mimeXml = buildMimeInfoXml({ fileAssociations: identity.fileAssociations });
  if (mimeXml) {
    if (!identity.id) throw new Error("nd: mimeType'd file associations need app.id (the shared-mime-info package name)");
    mkdirSync(join(appdir, "usr/share/mime/packages"), { recursive: true });
    writeFileSync(join(appdir, `usr/share/mime/packages/${identity.id}.xml`), mimeXml);
  }

  const hostBinary = await resolveHostBinary({ backend: "gtk" });
  const exe = join(appdir, "usr", "bin", identity.slug);
  cpSync(hostBinary, exe, { dereference: true });
  chmodSync(exe, 0o755);
  const bunPath = config.package?.bunPath ?? Bun.which("bun");
  if (!bunPath) throw new Error("nd: bun not found on PATH");
  // dereference: true - `bun` on PATH is frequently a nix-store symlink.
  cpSync(bunPath, join(appdir, "usr", "bin", "bun"), { dereference: true });
  chmodSync(join(appdir, "usr", "bin", "bun"), 0o755);

  let bundlePath = appdir;
  const format = options.format ?? linux?.format ?? "appimage";
  if (format === "appimage") {
    const appImage = join(dist, `${identity.name}-${identity.version}.AppImage`);
    await $`appimagetool ${appdir} ${appImage}`.quiet()
      .catch(async () => { await $`mksquashfs ${appdir} ${appImage} -root-owned -noappend`.quiet(); });
    console.error(`ND_PACKAGE_APPIMAGE ${appImage}`);
    bundlePath = appImage;
  }

  const update = await maybePublishUpdate(config.package?.updates, {
    identity,
    platform: "linux",
    distDir: dist,
    member: "AppDir",
    outDir,
  });

  console.error(`ND_PACKAGE_OK ${bundlePath}`);
  return { bundlePath, updateManifest: update?.manifestPath, publicKey: update?.publicKey };
}
