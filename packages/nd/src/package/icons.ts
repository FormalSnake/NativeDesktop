// App icon installation. mac: .icns/.iconset passthrough or a PNG source
// rasterized through sips + iconutil. linux: hicolor theme set + the AppDir
// root icon appimagetool requires. Resizers are probed, never hard-required:
// sips/iconutil are macOS-only tools, and a Linux box degrades with a marker.
import { cpSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { extname, join, resolve } from "node:path";
import type { ResolvedIdentity } from "./identity.ts";
import { PLACEHOLDER_PNG } from "./templates.ts";

async function run(command: string[]): Promise<void> {
  const proc = Bun.spawn(command, { stdin: "ignore", stdout: "inherit", stderr: "inherit" });
  if (await proc.exited !== 0) throw new Error(`nd: icon tool failed (${command.join(" ")})`);
}

let warnedNoIcon = false;
function warnNoIcon(): void {
  if (warnedNoIcon) return;
  warnedNoIcon = true;
  console.error("nd: no app icon configured (app.icon) - shipping the 1x1 placeholder");
}

/** The 10 filenames iconutil expects in a .iconset. */
const ICONSET_SIZES: Array<{ file: string; px: number }> = [
  { file: "icon_16x16.png", px: 16 },
  { file: "icon_16x16@2x.png", px: 32 },
  { file: "icon_32x32.png", px: 32 },
  { file: "icon_32x32@2x.png", px: 64 },
  { file: "icon_128x128.png", px: 128 },
  { file: "icon_128x128@2x.png", px: 256 },
  { file: "icon_256x256.png", px: 256 },
  { file: "icon_256x256@2x.png", px: 512 },
  { file: "icon_512x512.png", px: 512 },
  { file: "icon_512x512@2x.png", px: 1024 },
];

/**
 * Installs the mac icon into Contents/Resources/<slug>.icns. Returns the
 * CFBundleIconFile value, or undefined when no icon is configured.
 */
export async function installMacIcon(identity: ResolvedIdentity, appDir: string, resourcesDir: string): Promise<string | undefined> {
  const source = identity.icon?.macos ?? identity.icon?.source;
  if (!source) {
    warnNoIcon();
    return undefined;
  }
  const src = resolve(appDir, source);
  const ext = extname(src).toLowerCase();
  const icns = join(resourcesDir, `${identity.slug}.icns`);
  if (ext === ".icns") {
    cpSync(src, icns);
  } else if (ext === ".iconset") {
    await run(["iconutil", "-c", "icns", "-o", icns, src]);
  } else if (ext === ".png") {
    const tmp = mkdtempSync(join(tmpdir(), "nd-iconset-"));
    const iconset = join(tmp, `${identity.slug}.iconset`);
    mkdirSync(iconset, { recursive: true });
    for (const { file, px } of ICONSET_SIZES) {
      await run(["sips", "-z", String(px), String(px), src, "--out", join(iconset, file)]);
    }
    await run(["iconutil", "-c", "icns", "-o", icns, iconset]);
    rmSync(tmp, { recursive: true, force: true });
  } else if (ext === ".svg") {
    throw new Error(
      "nd: macOS cannot rasterize an SVG icon. Supply a 1024px PNG (app.icon.source) or a prebuilt .icns (app.icon.macos).",
    );
  } else {
    throw new Error(`nd: unsupported mac icon source "${source}" (want .png, .icns, or .iconset)`);
  }
  return identity.slug;
}

const HICOLOR_SIZES = [16, 32, 48, 64, 128, 256, 512];

function firstOnPath(candidates: string[]): string | undefined {
  return candidates.find((tool) => Bun.which(tool) !== null);
}

async function resizePng(tool: string, src: string, px: number, out: string): Promise<void> {
  switch (tool) {
    case "sips":
      await run(["sips", "-z", String(px), String(px), src, "--out", out]);
      return;
    case "magick":
      await run(["magick", src, "-resize", `${px}x${px}`, out]);
      return;
    case "convert":
      await run(["convert", src, "-resize", `${px}x${px}`, out]);
      return;
    case "rsvg-convert":
      await run(["rsvg-convert", "-w", String(px), "-h", String(px), "-o", out, src]);
      return;
    default:
      throw new Error(`nd: unknown icon resizer "${tool}"`);
  }
}

/** Installs the linux icon set into the AppDir (hicolor theme + root icon). */
export async function installLinuxIcon(identity: ResolvedIdentity, appDir: string, appdir: string): Promise<void> {
  const source = identity.icon?.linux ?? identity.icon?.source;
  const rootIcon = join(appdir, `${identity.slug}.png`);
  if (!source) {
    warnNoIcon();
    writeFileSync(rootIcon, PLACEHOLDER_PNG);
    return;
  }
  const src = resolve(appDir, source);
  const ext = extname(src).toLowerCase();
  if (ext === ".svg") {
    const scalable = join(appdir, "usr/share/icons/hicolor/scalable/apps");
    mkdirSync(scalable, { recursive: true });
    cpSync(src, join(scalable, `${identity.slug}.svg`));
    cpSync(src, join(appdir, `${identity.slug}.svg`));
    return;
  }
  if (ext !== ".png") throw new Error(`nd: unsupported linux icon source "${source}" (want .png or .svg)`);
  const resizer = firstOnPath(["sips", "magick", "convert", "rsvg-convert"]);
  if (!resizer) {
    console.error("ND_PACKAGE_ICON_SKIPPED reason=no-resizer");
    cpSync(src, rootIcon);
    return;
  }
  for (const px of HICOLOR_SIZES) {
    const dir = join(appdir, `usr/share/icons/hicolor/${px}x${px}/apps`);
    mkdirSync(dir, { recursive: true });
    await resizePng(resizer, src, px, join(dir, `${identity.slug}.png`));
  }
  cpSync(src, rootIcon);
}
