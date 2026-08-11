// Linux icon degrade paths: resizers are probed, never hard-required, and a
// broken resizer must not abort packaging. Each scenario runs in a subprocess
// whose PATH holds only the fixture's fake tools, because Bun.which and spawn
// resolve against the process's startup PATH.
import { describe, expect, test } from "bun:test";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

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
