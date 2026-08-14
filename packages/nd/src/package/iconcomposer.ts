// macOS 26 layered app icons. An Icon Composer ".icon" is a directory holding an
// icon.json manifest next to an Assets/ folder of layer art; actool compiles one
// into the Assets.car macOS 26 renders plus the .icns older releases fall back
// to. Two constraints shape the code here: actool matches --app-icon against the
// bundle's BASENAME and exits 0 having written nothing when they disagree, and
// icon.json colors must be "<space>:r,g,b,a" floats (a hex string is rejected).
//
// The same composition also flattens to a single SVG for Linux, so one set of
// layer art dresses both platforms.
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, extname, join, resolve } from "node:path";
import type { AppIconFill, AppIconLayer, AppIconLayered } from "../config.ts";

/** Runs an icon tool, surfacing its output only when it fails (actool prints a
 * result plist to stdout on every successful compile). */
export async function runTool(command: string[]): Promise<void> {
  const proc = Bun.spawn(command, { stdin: "ignore", stdout: "pipe", stderr: "pipe" });
  const [out, err, code] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
    proc.exited,
  ]);
  if (code === 0) return;
  if (out) process.stderr.write(out);
  if (err) process.stderr.write(err);
  throw new Error(`nd: icon tool failed (${command.join(" ")})`);
}

type Rgba = [number, number, number, number];

/** Accepts #rgb/#rgba/#rrggbb/#rrggbbaa or an Icon Composer "<space>:r,g,b,a" string. */
function parseColor(value: string): Rgba {
  const colon = value.indexOf(":");
  if (colon >= 0) {
    const parts = value.slice(colon + 1).split(",").map(Number);
    if (parts.length < 3 || parts.some(Number.isNaN)) throw new Error(`nd: unreadable icon color "${value}"`);
    return [parts[0]!, parts[1]!, parts[2]!, parts[3] ?? 1];
  }
  const hex = value.startsWith("#") ? value.slice(1) : value;
  const wide = hex.length <= 4 ? hex.replace(/./g, (c) => c + c) : hex;
  if (!/^[0-9a-f]{6}([0-9a-f]{2})?$/i.test(wide)) {
    throw new Error(`nd: unsupported icon color "${value}" (want #rrggbb, #rrggbbaa, or "display-p3:r,g,b,a")`);
  }
  const channel = (at: number) => parseInt(wide.slice(at, at + 2), 16) / 255;
  return [channel(0), channel(2), channel(4), wide.length === 8 ? channel(6) : 1];
}

/** A color string icon.json accepts. Color-space inputs pass through untouched;
 * hex is normalized into extended sRGB floats. */
function iconColor(value: string): string {
  if (value.includes(":")) return value;
  return `extended-srgb:${parseColor(value).map((c) => c.toFixed(5)).join(",")}`;
}

function cssColor(value: string): string {
  const byte = (c: number) => Math.round(Math.min(1, Math.max(0, c)) * 255).toString(16).padStart(2, "0");
  const [r, g, b] = parseColor(value);
  return `#${byte(r)}${byte(g)}${byte(b)}`;
}

function cssOpacity(value: string): number {
  return Math.min(1, Math.max(0, parseColor(value)[3]));
}

function fillManifest(fill: AppIconFill): Record<string, unknown> {
  if (typeof fill === "string") return { solid: iconColor(fill) };
  const [top, bottom] = fill.gradient;
  return { "linear-gradient": [iconColor(top), iconColor(bottom)] };
}

const LAYER_EXTENSIONS = new Set([".svg", ".png"]);
/** Icon Composer stacks at most four groups, one per depth level. */
const MAX_LAYERS = 4;

interface ResolvedLayer {
  path: string;
  /** Filename inside Assets/, deduplicated across layers from different directories. */
  file: string;
  name: string;
  layer: AppIconLayer;
}

function resolveLayers(layered: AppIconLayered, appDir: string): ResolvedLayer[] {
  const entries = layered.layers ?? [];
  if (!entries.length) throw new Error("nd: app.icon.layered needs at least one layer");
  if (entries.length > MAX_LAYERS) {
    throw new Error(`nd: app.icon.layered takes at most ${MAX_LAYERS} layers (got ${entries.length})`);
  }
  const taken = new Set<string>();
  return entries.map((entry) => {
    const layer = typeof entry === "string" ? { image: entry } : entry;
    const path = resolve(appDir, layer.image);
    const ext = extname(path).toLowerCase();
    if (!LAYER_EXTENSIONS.has(ext)) throw new Error(`nd: unsupported icon layer "${layer.image}" (want .svg or .png)`);
    if (!existsSync(path)) throw new Error(`nd: icon layer not found: ${path}`);
    const stem = basename(path, ext);
    let file = `${stem}${ext}`;
    for (let n = 2; taken.has(file); n++) file = `${stem}-${n}${ext}`;
    taken.add(file);
    return { path, file, name: layer.name ?? stem, layer };
  });
}

/** The shape readIconBundle returns, for a layered config instead of a bundle. */
export function layeredComposition(layered: AppIconLayered, appDir: string): { background?: AppIconFill; layers: string[] } {
  return { background: layered.background, layers: resolveLayers(layered, appDir).map((layer) => layer.path) };
}

/** The icon.json a layered config compiles to. */
export function iconManifest(layered: AppIconLayered, appDir: string): string {
  return manifestJson(layered, resolveLayers(layered, appDir));
}

/** One group per layer, so the glass controls stay per-layer in the config. */
function manifestJson(layered: AppIconLayered, layers: ResolvedLayer[]): string {
  const manifest: Record<string, unknown> = {};
  if (layered.background) manifest.fill = fillManifest(layered.background);
  manifest.groups = layers.map(({ file, name, layer }) => ({
    layers: [{ "image-name": file, name }],
    specular: layer.specular ?? true,
    translucency: layer.translucency === false
      ? { enabled: false, value: 0 }
      : { enabled: true, value: layer.translucency ?? 0.5 },
    shadow: layer.shadow === false
      ? { kind: "none", opacity: 0 }
      : { kind: "neutral", opacity: layer.shadow ?? 0.5 },
  }));
  return `${JSON.stringify(manifest, null, 2)}\n`;
}

/** Writes a .icon bundle at `bundle` from a layered config. */
export function writeIconBundle(layered: AppIconLayered, appDir: string, bundle: string): void {
  const assets = join(bundle, "Assets");
  mkdirSync(assets, { recursive: true });
  const layers = resolveLayers(layered, appDir);
  for (const { path, file } of layers) cpSync(path, join(assets, file));
  writeFileSync(join(bundle, "icon.json"), manifestJson(layered, layers));
}

/**
 * Compiles a .icon into Assets.car + the legacy .icns inside `resourcesDir`.
 * Returns the CFBundleIconName/CFBundleIconFile value.
 */
export async function compileIconBundle(bundle: string, resourcesDir: string, deploymentTarget: string): Promise<string> {
  if (!Bun.which("xcrun")) {
    throw new Error(`nd: compiling ${basename(bundle)} needs Xcode's actool (install Xcode, or set app.icon.macos to a prebuilt .icns)`);
  }
  const name = basename(bundle, ".icon");
  const out = mkdtempSync(join(tmpdir(), "nd-actool-"));
  try {
    await runTool([
      "xcrun", "actool", bundle,
      "--app-icon", name,
      "--compile", out,
      "--output-partial-info-plist", join(out, "partial.plist"),
      "--minimum-deployment-target", deploymentTarget,
      "--platform", "macosx",
      "--target-device", "mac",
    ]);
    const car = join(out, "Assets.car");
    // actool keys off the bundle basename and reports success while emitting
    // nothing when the icon it was asked for isn't there, so a missing
    // Assets.car is the only signal that the compile didn't take.
    if (!existsSync(car)) throw new Error(`nd: actool compiled no icon out of ${bundle}`);
    cpSync(car, join(resourcesDir, "Assets.car"));
    const icns = join(out, `${name}.icns`);
    if (existsSync(icns)) cpSync(icns, join(resourcesDir, `${name}.icns`));
    return name;
  } finally {
    rmSync(out, { recursive: true, force: true });
  }
}

/** Icon Composer writes richer fills than the Linux flatten can draw, so an
 * unreadable one is an error naming the escape hatch rather than a wrong icon. */
function readFill(fill: Record<string, unknown> | undefined, bundle: string): AppIconFill | undefined {
  if (!fill) return undefined;
  const gradient = fill["linear-gradient"];
  if (Array.isArray(gradient) && gradient.length >= 2 && gradient.every((stop) => typeof stop === "string")) {
    return { gradient: [gradient[0] as string, gradient[1] as string] };
  }
  // An automatic gradient is derived by the renderer from one seed color; Linux
  // has no equivalent, so it flattens to that color.
  const solid = fill.solid ?? fill["automatic-gradient"];
  if (typeof solid === "string") return solid;
  throw new Error(`nd: cannot flatten ${bundle}'s background for Linux - set app.icon.linux to a PNG or SVG`);
}

/** The parts of an existing .icon the Linux flatten needs: the background fill
 * and every layer's art, back to front. */
export function readIconBundle(bundle: string): { background?: AppIconFill; layers: string[] } {
  const manifestPath = join(bundle, "icon.json");
  if (!existsSync(manifestPath)) throw new Error(`nd: ${bundle} has no icon.json`);
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8")) as {
    fill?: Record<string, unknown>;
    groups?: Array<{ layers?: Array<{ "image-name"?: string }> }>;
  };
  const background = readFill(manifest.fill, basename(bundle));
  const layers: string[] = [];
  for (const group of manifest.groups ?? []) {
    for (const layer of group.layers ?? []) {
      const file = layer["image-name"];
      if (!file) continue;
      const path = join(bundle, "Assets", file);
      if (!existsSync(path)) throw new Error(`nd: ${basename(bundle)} references a missing layer: Assets/${file}`);
      layers.push(path);
    }
  }
  if (!layers.length) throw new Error(`nd: ${basename(bundle)} declares no layers`);
  return { background, layers };
}

/** Icon Composer's design grid. */
const GRID = 1024;
// GNOME draws an app icon as a 104x104 body on a 128x128 canvas with a 24px
// corner radius. Mapped onto the 1024 grid that is an 832px body inset by 96
// with a 192px radius, which is what lets one composition serve both platforms.
const BODY = 832;
const INSET = 96;
const RADIUS = 192;

/** Flattens a layered composition into one SVG sized to the GNOME icon grid. */
export function composeSvg(background: AppIconFill | undefined, layers: string[]): string {
  const defs: string[] = [];
  let fill = "none";
  let fillOpacity = 1;
  if (typeof background === "string") {
    fill = cssColor(background);
    fillOpacity = cssOpacity(background);
  } else if (background) {
    const [top, bottom] = background.gradient;
    defs.push(
      `<linearGradient id="nd-background" x1="0" y1="0" x2="0" y2="1">`
      + `<stop offset="0" stop-color="${cssColor(top)}" stop-opacity="${cssOpacity(top)}"/>`
      + `<stop offset="1" stop-color="${cssColor(bottom)}" stop-opacity="${cssOpacity(bottom)}"/>`
      + `</linearGradient>`,
    );
    fill = "url(#nd-background)";
  }
  const art = layers.map((path, index) => inlineLayer(path, `nd${index}`)).join("");
  return [
    `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"`
    + ` width="${GRID}" height="${GRID}" viewBox="0 0 ${GRID} ${GRID}">`,
    defs.length ? `<defs>${defs.join("")}</defs>` : "",
    background
      ? `<rect x="${INSET}" y="${INSET}" width="${BODY}" height="${BODY}" rx="${RADIUS}" ry="${RADIUS}"`
        + ` fill="${fill}" fill-opacity="${fillOpacity}"/>`
      : "",
    `<g transform="translate(${INSET} ${INSET}) scale(${BODY / GRID})">${art}</g>`,
    `</svg>`,
    "",
  ].join("\n");
}

function inlineLayer(path: string, prefix: string): string {
  if (extname(path).toLowerCase() === ".png") {
    const data = readFileSync(path).toString("base64");
    const href = `data:image/png;base64,${data}`;
    return `<image x="0" y="0" width="${GRID}" height="${GRID}" preserveAspectRatio="xMidYMid meet"`
      + ` href="${href}" xlink:href="${href}"/>`;
  }
  return nestSvg(readFileSync(path, "utf8"), prefix, path);
}

const ATTRIBUTE = `\\s*=\\s*("[^"]*"|'[^']*')`;

/**
 * Inlines an SVG file as a nested <svg> on the 1024 grid. Nested <svg> is plain
 * SVG 1.1 that every renderer handles, unlike an SVG data URI inside <image>,
 * which only librsvg-backed rasterizers read. Ids stay document-global through
 * the nesting, so each layer's are namespaced before the layers are merged.
 */
function nestSvg(source: string, prefix: string, path: string): string {
  const document = source.replace(/<\?xml[\s\S]*?\?>/g, "").replace(/<!DOCTYPE[\s\S]*?>/gi, "").trim();
  const open = document.match(/<svg\b([^>]*)>/i);
  const close = document.lastIndexOf("</svg>");
  if (!open || close < 0) throw new Error(`nd: icon layer is not an SVG document: ${path}`);
  const attributes = open[1]!;
  const viewBox = attribute(attributes, "viewBox") ?? intrinsicViewBox(attributes);
  const kept = attributes.replace(
    new RegExp(`\\s(?:width|height|viewBox|x|y|preserveAspectRatio)${ATTRIBUTE}`, "gi"),
    "",
  );
  const inner = document.slice(open.index! + open[0].length, close);
  return `<svg${kept} x="0" y="0" width="${GRID}" height="${GRID}" viewBox="${viewBox}"`
    + ` preserveAspectRatio="xMidYMid meet">${namespaceIds(inner, prefix)}</svg>`;
}

function attribute(attributes: string, name: string): string | undefined {
  const match = attributes.match(new RegExp(`\\s${name}${ATTRIBUTE}`, "i"));
  return match ? match[1]!.slice(1, -1) : undefined;
}

/** An SVG without a viewBox is measured by its width/height, dropping any unit suffix. */
function intrinsicViewBox(attributes: string): string {
  const width = Number.parseFloat(attribute(attributes, "width") ?? "");
  const height = Number.parseFloat(attribute(attributes, "height") ?? "");
  if (!Number.isFinite(width) || !Number.isFinite(height)) return `0 0 ${GRID} ${GRID}`;
  return `0 0 ${width} ${height}`;
}

function namespaceIds(svg: string, prefix: string): string {
  const ids = new Set<string>();
  for (const [, id] of svg.matchAll(/\sid\s*=\s*["']([^"']+)["']/g)) ids.add(id!);
  let out = svg;
  for (const id of ids) {
    const escaped = id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    out = out
      .replace(new RegExp(`(\\sid\\s*=\\s*["'])${escaped}(["'])`, "g"), `$1${prefix}-${id}$2`)
      .replace(new RegExp(`url\\((["']?)#${escaped}\\1\\)`, "g"), `url($1#${prefix}-${id}$1)`)
      .replace(new RegExp(`((?:xlink:)?href\\s*=\\s*["'])#${escaped}(["'])`, "g"), `$1#${prefix}-${id}$2`);
  }
  return out;
}
