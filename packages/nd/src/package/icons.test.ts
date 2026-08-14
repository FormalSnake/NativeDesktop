// Linux icon degrade paths: resizers are probed, never hard-required, and a
// broken resizer must not abort packaging. Each scenario runs in a subprocess
// whose PATH holds only the fixture's fake tools, because Bun.which and spawn
// resolve against the process's startup PATH.
import { describe, expect, test } from "bun:test";
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { AppIconLayered, AppIcon } from "../config.ts";
import { installLinuxIcon, installMacIcon } from "./icons.ts";
import { composeSvg, iconManifest, layeredComposition } from "./iconcomposer.ts";
import type { ResolvedIdentity } from "./identity.ts";
import { PLACEHOLDER_PNG } from "./templates.ts";

const ICONS_MODULE = join(import.meta.dir, "icons.ts");

function fixture(tools: (root: string) => Record<string, string>): { root: string; appDir: string; appdir: string; bin: string } {
  const root = mkdtempSync(join(tmpdir(), "nd-icons-"));
  const appDir = join(root, "app");
  const appdir = join(root, "AppDir");
  mkdirSync(appDir, { recursive: true });
  mkdirSync(appdir, { recursive: true });
  writeFileSync(join(appDir, "icon.png"), "png bytes");
  const bin = join(root, "bin");
  mkdirSync(bin, { recursive: true });
  for (const [name, script] of Object.entries(tools(root))) {
    const file = join(bin, name);
    writeFileSync(file, `#!/bin/sh\n${script}\n`);
    chmodSync(file, 0o755);
  }
  return { root, appDir, appdir, bin };
}

function runInstall(appDir: string, appdir: string, path: string): { exitCode: number; stderr: string } {
  const script = `
    import { installLinuxIcon } from ${JSON.stringify(ICONS_MODULE)};
    await installLinuxIcon(
      { name: "Fixture", displayName: "Fixture", slug: "fixture", version: "1.0.0", categories: ["Utility"], icon: { source: "icon.png" } },
      ${JSON.stringify(appDir)},
      ${JSON.stringify(appdir)},
    );
  `;
  const proc = Bun.spawnSync([process.execPath, "-e", script], { env: { PATH: path } });
  return { exitCode: proc.exitCode, stderr: proc.stderr.toString() };
}

describe("installLinuxIcon", () => {
  test("a failing resizer degrades to the root icon instead of aborting", () => {
    const { appDir, appdir, bin } = fixture(() => ({ sips: "exit 1" }));
    const { exitCode, stderr } = runInstall(appDir, appdir, bin);
    expect(exitCode).toBe(0);
    expect(stderr).toContain("ND_PACKAGE_ICON_SKIPPED reason=resize-failed");
    expect(existsSync(join(appdir, "fixture.png"))).toBe(true);
    expect(existsSync(join(appdir, "usr/share/icons/hicolor"))).toBe(false);
  });

  test("rsvg-convert is never probed for a PNG source", () => {
    const { root, appDir, appdir, bin } = fixture((r) => ({ "rsvg-convert": `touch "${r}/invoked"; exit 1` }));
    const { exitCode, stderr } = runInstall(appDir, appdir, bin);
    expect(exitCode).toBe(0);
    expect(stderr).toContain("ND_PACKAGE_ICON_SKIPPED reason=no-resizer");
    expect(existsSync(join(root, "invoked"))).toBe(false);
    expect(existsSync(join(appdir, "fixture.png"))).toBe(true);
    expect(existsSync(join(appdir, "usr/share/icons/hicolor"))).toBe(false);
  });
});

const GLASS_SVG = `<?xml version="1.0"?>\n<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512">`
  + `<defs><linearGradient id="g"><stop offset="0" stop-color="#fff"/></linearGradient></defs>`
  + `<circle cx="256" cy="256" r="200" fill="url(#g)"/></svg>`;

function layeredFixture(layered: AppIconLayered): { appDir: string; identity: ResolvedIdentity } {
  const appDir = mkdtempSync(join(tmpdir(), "nd-layered-"));
  writeFileSync(join(appDir, "glyph.svg"), GLASS_SVG);
  writeFileSync(join(appDir, "shine.png"), PLACEHOLDER_PNG);
  return { appDir, identity: identityWith({ layered }) };
}

function identityWith(icon: AppIcon): ResolvedIdentity {
  return { name: "Fixture", displayName: "Fixture", slug: "fixture", version: "1.0.0", categories: ["Utility"], icon };
}

describe("iconManifest", () => {
  test("hex background becomes an extended-sRGB gradient and each layer gets its own group", () => {
    const { appDir } = layeredFixture({ layers: [] });
    const manifest = JSON.parse(iconManifest(
      { background: { gradient: ["#0a84ff", "#5e5ce6"] }, layers: ["glyph.svg", { image: "shine.png", shadow: false, translucency: 0.2 }] },
      appDir,
    ));
    expect(manifest.fill["linear-gradient"]).toEqual(["extended-srgb:0.03922,0.51765,1.00000,1.00000", "extended-srgb:0.36863,0.36078,0.90196,1.00000"]);
    expect(manifest.groups).toHaveLength(2);
    expect(manifest.groups[0].layers[0]).toEqual({ "image-name": "glyph.svg", name: "glyph" });
    expect(manifest.groups[0].shadow).toEqual({ kind: "neutral", opacity: 0.5 });
    expect(manifest.groups[1].shadow).toEqual({ kind: "none", opacity: 0 });
    expect(manifest.groups[1].translucency).toEqual({ enabled: true, value: 0.2 });
  });

  test("a color-space string passes through untouched", () => {
    const { appDir } = layeredFixture({ layers: [] });
    const manifest = JSON.parse(iconManifest({ background: "display-p3:0,0.5,1,1", layers: ["glyph.svg"] }, appDir));
    expect(manifest.fill).toEqual({ solid: "display-p3:0,0.5,1,1" });
  });

  test("more than four layers is rejected", () => {
    const { appDir } = layeredFixture({ layers: [] });
    const layers = ["glyph.svg", "glyph.svg", "glyph.svg", "glyph.svg", "glyph.svg"];
    expect(() => iconManifest({ layers }, appDir)).toThrow(/at most 4 layers/);
  });
});

describe("composeSvg", () => {
  test("layers nest as SVG with namespaced ids over the GNOME icon body", () => {
    const { appDir } = layeredFixture({ layers: [] });
    const layered: AppIconLayered = { background: "#0a84ff", layers: ["glyph.svg", "shine.png"] };
    const { background, layers } = layeredComposition(layered, appDir);
    const svg = composeSvg(background, layers);
    expect(svg).toContain(`<rect x="96" y="96" width="832" height="832" rx="192"`);
    expect(svg).toContain(`fill="#0a84ff"`);
    // The layer's own gradient id is rewritten on both the definition and the reference.
    expect(svg).toContain(`id="nd0-g"`);
    expect(svg).toContain(`url(#nd0-g)`);
    // The nested <svg> keeps the source viewBox but is stretched onto the 1024 grid.
    expect(svg).toContain(`viewBox="0 0 512 512" preserveAspectRatio="xMidYMid meet"`);
    expect(svg).toContain(`data:image/png;base64,`);
    expect(svg).not.toContain("<?xml version");
  });
});

const rasterizer = ["rsvg-convert", "magick", "convert"].some((tool) => Bun.which(tool) !== null);

function iconBundleFixture(fill: unknown): string {
  const appDir = mkdtempSync(join(tmpdir(), "nd-bundle-"));
  mkdirSync(join(appDir, "Brand.icon", "Assets"), { recursive: true });
  writeFileSync(join(appDir, "Brand.icon", "Assets", "glyph.svg"), GLASS_SVG);
  writeFileSync(join(appDir, "Brand.icon", "icon.json"), JSON.stringify({
    fill,
    groups: [{ layers: [{ "image-name": "glyph.svg", name: "glyph" }] }],
  }));
  return appDir;
}

describe.if(rasterizer)("installLinuxIcon", () => {
  test("an Icon Composer bundle flattens into the linux icon set", async () => {
    const appDir = iconBundleFixture({ "linear-gradient": ["extended-srgb:0,0.5,1,1", "extended-srgb:0.4,0.3,0.9,1"] });
    const appdir = mkdtempSync(join(tmpdir(), "nd-appdir-"));
    await installLinuxIcon(identityWith({ source: "Brand.icon" }), appDir, appdir);
    expect(existsSync(join(appdir, "usr/share/icons/hicolor/scalable/apps/fixture.svg"))).toBe(true);
    expect(existsSync(join(appdir, "usr/share/icons/hicolor/256x256/apps/fixture.png"))).toBe(true);
  });

  test("a fill the flatten cannot draw names the escape hatch", async () => {
    const appDir = iconBundleFixture({ "automatic-gradient": { seed: "extended-srgb:0,0.5,1,1" } });
    const appdir = mkdtempSync(join(tmpdir(), "nd-appdir-"));
    await expect(installLinuxIcon(identityWith({ source: "Brand.icon" }), appDir, appdir)).rejects.toThrow(/app\.icon\.linux/);
  });

  test("the linux override beats a layered composition", async () => {
    const { appDir, identity } = layeredFixture({ background: "#0a84ff", layers: ["glyph.svg"] });
    const appdir = mkdtempSync(join(tmpdir(), "nd-appdir-"));
    await installLinuxIcon(identityWith({ layered: identity.icon!.layered, linux: "shine.png" }), appDir, appdir);
    // The override is a PNG, so the flatten never ran and there is no scalable icon.
    expect(existsSync(join(appdir, "usr/share/icons/hicolor/scalable/apps/fixture.svg"))).toBe(false);
    expect(existsSync(join(appdir, "fixture.png"))).toBe(true);
  });

  test("a layered config flattens to the scalable icon plus the hicolor raster set", async () => {
    const { appDir, identity } = layeredFixture({ background: { gradient: ["#0a84ff", "#5e5ce6"] }, layers: ["glyph.svg"] });
    const appdir = mkdtempSync(join(tmpdir(), "nd-appdir-"));
    await installLinuxIcon(identity, appDir, appdir);
    expect(existsSync(join(appdir, "usr/share/icons/hicolor/scalable/apps/fixture.svg"))).toBe(true);
    expect(existsSync(join(appdir, "usr/share/icons/hicolor/512x512/apps/fixture.png"))).toBe(true);
    expect(existsSync(join(appdir, "usr/share/icons/hicolor/16x16/apps/fixture.png"))).toBe(true);
    expect(existsSync(join(appdir, "fixture.svg"))).toBe(true);
  });
});

const actool = process.platform === "darwin" && Bun.which("xcrun") !== null;

describe.if(actool)("installMacIcon", () => {
  test("a layered config compiles to Assets.car plus the legacy icns", async () => {
    const { appDir, identity } = layeredFixture({ background: "#0a84ff", layers: ["glyph.svg"] });
    const resources = mkdtempSync(join(tmpdir(), "nd-resources-"));
    const iconFile = await installMacIcon(identity, appDir, resources, "26.0");
    expect(iconFile).toBe("fixture");
    expect(existsSync(join(resources, "Assets.car"))).toBe(true);
    expect(readFileSync(join(resources, "fixture.icns")).subarray(0, 4).toString()).toBe("icns");
  });

  test("the macos override beats a layered composition", async () => {
    const { appDir, identity } = layeredFixture({ background: "#0a84ff", layers: ["glyph.svg"] });
    const prebuilt = mkdtempSync(join(tmpdir(), "nd-resources-"));
    await installMacIcon(identityWith({ layered: identity.icon!.layered }), appDir, prebuilt, "26.0");
    cpSync(join(prebuilt, "fixture.icns"), join(appDir, "brand.icns"));

    const resources = mkdtempSync(join(tmpdir(), "nd-resources-"));
    const iconFile = await installMacIcon(
      identityWith({ layered: identity.icon!.layered, macos: "brand.icns" }),
      appDir,
      resources,
      "26.0",
    );
    expect(iconFile).toBe("fixture");
    // The override is a flat icns, so nothing was compiled through actool.
    expect(existsSync(join(resources, "Assets.car"))).toBe(false);
  });

  test("an SVG source rasterizes into a flat icns", async () => {
    const appDir = mkdtempSync(join(tmpdir(), "nd-flat-"));
    writeFileSync(join(appDir, "icon.svg"), GLASS_SVG);
    const resources = mkdtempSync(join(tmpdir(), "nd-resources-"));
    const iconFile = await installMacIcon(identityWith({ source: "icon.svg" }), appDir, resources, "26.0");
    expect(iconFile).toBe("fixture");
    expect(readFileSync(join(resources, "fixture.icns")).subarray(0, 4).toString()).toBe("icns");
    expect(existsSync(join(resources, "Assets.car"))).toBe(false);
  });
});
