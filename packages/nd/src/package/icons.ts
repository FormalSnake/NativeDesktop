// App icon installation. One source per platform is resolved from app.icon and
// converted to whatever the platform actually reads: mac gets Assets.car +
// .icns, linux gets the hicolor theme set plus the AppDir root icon
// appimagetool requires. Resizers are probed, never hard-required: sips and
// iconutil are macOS-only, and a Linux box degrades with a marker.
import { cpSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, extname, join, resolve } from "node:path";
import {
  compileIconBundle,
  composeSvg,
  layeredComposition,
  readIconBundle,
  runTool,
  writeIconBundle,
} from "./iconcomposer.ts";
import type { ResolvedIdentity } from "./identity.ts";
import { PLACEHOLDER_PNG } from "./templates.ts";

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

const HICOLOR_SIZES = [16, 32, 48, 64, 128, 256, 512];

function firstOnPath(candidates: string[]): string | undefined {
  return candidates.find((tool) => Bun.which(tool) !== null);
}

async function resize(tool: string, src: string, px: number, out: string): Promise<void> {
  switch (tool) {
    case "sips":
      await runTool(["sips", "-z", String(px), String(px), src, "--out", out]);
      return;
    case "magick":
      await runTool(["magick", src, "-resize", `${px}x${px}`, out]);
      return;
    case "convert":
      await runTool(["convert", src, "-resize", `${px}x${px}`, out]);
      return;
    case "rsvg-convert":
      await runTool(["rsvg-convert", "-w", String(px), "-h", String(px), "-o", out, src]);
      return;
    default:
      throw new Error(`nd: unknown icon resizer "${tool}"`);
  }
}

/**
 * Rasterizes an SVG to a square PNG. qlmanage is the macOS fallback: it ships
 * with the OS and renders through QuickLook, writing <name>.svg.png into the
 * output directory.
 */
async function rasterizeSvg(src: string, px: number, workDir: string): Promise<string> {
  const tool = firstOnPath(["rsvg-convert", "magick", "convert"]);
  if (tool) {
    const out = join(workDir, "source.png");
    await resize(tool, src, px, out);
    return out;
  }
  if (process.platform === "darwin") {
    await runTool(["qlmanage", "-t", "-s", String(px), "-o", workDir, src]);
    const rendered = join(workDir, `${basename(src)}.png`);
    if (existsSync(rendered)) return rendered;
  }
  throw new Error(`nd: cannot rasterize "${src}" (install librsvg or ImageMagick, or supply a 1024px PNG)`);
}

/** PNG -> .icns through a generated .iconset. */
async function buildIcns(src: string, icns: string, workDir: string): Promise<void> {
  const iconset = join(workDir, `${basename(icns, ".icns")}.iconset`);
  mkdirSync(iconset, { recursive: true });
  for (const { file, px } of ICONSET_SIZES) {
    await runTool(["sips", "-z", String(px), String(px), src, "--out", join(iconset, file)]);
  }
  await runTool(["iconutil", "-c", "icns", "-o", icns, iconset]);
}

/**
 * Installs the mac icon into Contents/Resources. A layered source compiles to
 * Assets.car (macOS 26 Liquid Glass) plus the .icns older releases fall back
 * to; a flat source compiles to <slug>.icns only. Returns the
 * CFBundleIconFile/CFBundleIconName value, or undefined when no icon is
 * configured.
 */
export async function installMacIcon(
  identity: ResolvedIdentity,
  appDir: string,
  resourcesDir: string,
  deploymentTarget = "26.0",
): Promise<string | undefined> {
  const override = identity.icon?.macos;
  // The explicit per-platform override wins outright. Failing that macOS
  // prefers a layered composition over the shared flat source, because layered
  // is its native format; Linux resolves the mirror of this.
  const layered = override ? undefined : identity.icon?.layered;
  const source = override ?? (layered ? undefined : identity.icon?.source);
  if (!layered && !source) {
    warnNoIcon();
    return undefined;
  }
  const work = mkdtempSync(join(tmpdir(), "nd-icon-"));
  try {
    if (layered) {
      const bundle = join(work, `${identity.slug}.icon`);
      writeIconBundle(layered, appDir, bundle);
      return await compileIconBundle(bundle, resourcesDir, deploymentTarget);
    }
    const src = resolve(appDir, source!);
    const ext = extname(src).toLowerCase();
    if (ext === ".icon") return await compileIconBundle(src, resourcesDir, deploymentTarget);
    const icns = join(resourcesDir, `${identity.slug}.icns`);
    switch (ext) {
      case ".icns":
        cpSync(src, icns);
        break;
      case ".iconset":
        await runTool(["iconutil", "-c", "icns", "-o", icns, src]);
        break;
      case ".png":
        await buildIcns(src, icns, work);
        break;
      case ".svg":
        await buildIcns(await rasterizeSvg(src, 1024, work), icns, work);
        break;
      default:
        throw new Error(`nd: unsupported mac icon source "${source}" (want .icon, .icns, .iconset, .png, or .svg)`);
    }
    return identity.slug;
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

/** Installs the linux icon set into the AppDir (hicolor theme + root icon). */
export async function installLinuxIcon(identity: ResolvedIdentity, appDir: string, appdir: string): Promise<void> {
  const source = identity.icon?.linux ?? identity.icon?.source;
  const layered = identity.icon?.layered;
  const rootIcon = join(appdir, `${identity.slug}.png`);
  if (!source && !layered) {
    warnNoIcon();
    writeFileSync(rootIcon, PLACEHOLDER_PNG);
    return;
  }
  const work = mkdtempSync(join(tmpdir(), "nd-icon-"));
  try {
    let src = source ? resolve(appDir, source) : "";
    let ext = src ? extname(src).toLowerCase() : "";
    // A layered composition (config or an Icon Composer bundle) flattens to a
    // single SVG so the same art dresses both platforms.
    if (!src || ext === ".icon") {
      const composition = src ? readIconBundle(src) : layeredComposition(layered!, appDir);
      src = join(work, `${identity.slug}.svg`);
      writeFileSync(src, composeSvg(composition.background, composition.layers));
      ext = ".svg";
    }
    if (ext === ".svg") {
      // Raster sizes first: a failed resizer clears the whole hicolor tree, and
      // scalable/ is a complete install on its own, so it lands afterwards.
      await installHicolor(identity, src, appdir, ["rsvg-convert", "magick", "convert"]);
      const scalable = join(appdir, "usr/share/icons/hicolor/scalable/apps");
      mkdirSync(scalable, { recursive: true });
      cpSync(src, join(scalable, `${identity.slug}.svg`));
      cpSync(src, join(appdir, `${identity.slug}.svg`));
      return;
    }
    if (ext !== ".png") throw new Error(`nd: unsupported linux icon source "${source}" (want .png, .svg, or .icon)`);
    // rsvg-convert is deliberately absent: it only rasterizes SVGs and this
    // branch always feeds it a PNG.
    if (!await installHicolor(identity, src, appdir, ["sips", "magick", "convert"])) {
      console.error("ND_PACKAGE_ICON_SKIPPED reason=no-resizer");
    }
    cpSync(src, rootIcon);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

/** Renders the hicolor raster set. Returns false when no resizer is installed,
 * which the caller reports only when raster is the sole install path. */
async function installHicolor(identity: ResolvedIdentity, src: string, appdir: string, tools: string[]): Promise<boolean> {
  const resizer = firstOnPath(tools);
  if (!resizer) return false;
  try {
    for (const px of HICOLOR_SIZES) {
      const dir = join(appdir, `usr/share/icons/hicolor/${px}x${px}/apps`);
      mkdirSync(dir, { recursive: true });
      await resize(resizer, src, px, join(dir, `${identity.slug}.png`));
    }
  } catch (err) {
    // A broken resizer degrades like a missing one: drop the (possibly
    // partial) hicolor set and ship the root icon only.
    rmSync(join(appdir, "usr/share/icons/hicolor"), { recursive: true, force: true });
    console.error(String(err));
    console.error("ND_PACKAGE_ICON_SKIPPED reason=resize-failed");
  }
  return true;
}
