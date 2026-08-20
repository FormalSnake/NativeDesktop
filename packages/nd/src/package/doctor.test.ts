// The two CEF doctor checks: a config that asks for chromium with no dist
// resolvable, and the zero-Chromium-bytes claim an engine="system" bundle makes.
import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { NativeDesktopConfig } from "../config.ts";
import { builtBundles, type Check, webviewChecks } from "./doctor.ts";

const FRAMEWORK = "Chromium Embedded Framework.framework";
const target = process.platform === "darwin" ? "mac" : "linux";

function tempApp(): string {
  return mkdtempSync(join(tmpdir(), "nd-doctor-"));
}

function find(checks: Check[], name: string): Check[] {
  return checks.filter((c) => c.name === name);
}

/** Stage a packaged bundle of the host platform's shape under <app>/dist. */
function stageBundle(app: string, withCef: boolean): string {
  if (process.platform === "darwin") {
    const bundle = join(app, "dist", "mac", "Demo.app");
    mkdirSync(join(bundle, "Contents", "MacOS"), { recursive: true });
    if (withCef) mkdirSync(join(bundle, "Contents", "Frameworks", FRAMEWORK), { recursive: true });
    return bundle;
  }
  const bundle = join(app, "dist", "linux", "AppDir");
  mkdirSync(join(bundle, "usr", "bin"), { recursive: true });
  if (withCef) {
    mkdirSync(join(bundle, "lib", "cef"), { recursive: true });
    writeFileSync(join(bundle, "lib", "cef", "libcef.so"), "");
  }
  return bundle;
}

describe("builtBundles", () => {
  test("finds what a previous package run left in the output dir", () => {
    const app = tempApp();
    const bundle = stageBundle(app, false);
    expect(builtBundles({}, app)).toEqual([bundle]);
    expect(builtBundles({}, tempApp())).toEqual([]);
  });
});

describe("webviewChecks", () => {
  test("reports the resolved engine for this platform", () => {
    const checks = webviewChecks({}, tempApp());
    expect(find(checks, "webview")[0]!.detail).toBe(`engine=system (${target})`);
    expect(find(checks, "cef")).toEqual([]);
  });

  test("chromium with no resolvable dist is an error naming the cache path", () => {
    const config: NativeDesktopConfig = {
      webview: { engine: { [target]: "chromium" }, cef: { version: "0.0.0-absent" } },
    };
    const cef = find(webviewChecks(config, tempApp()), "cef");
    const failure = cef.find((c) => c.status === "error");
    expect(failure).toBeDefined();
    expect(failure!.detail).toContain("0.0.0-absent");
    expect(failure!.detail).toContain("ND_CEF_ROOT");
  });

  test("an engine=system bundle carrying CEF fails the zero-bytes claim", () => {
    const clean = webviewChecks({}, (() => { const a = tempApp(); stageBundle(a, false); return a; })());
    expect(find(clean, "cef-bytes")[0]!.status).toBe("ok");

    const dirty = webviewChecks({}, (() => { const a = tempApp(); stageBundle(a, true); return a; })());
    expect(find(dirty, "cef-bytes")[0]!.status).toBe("error");
    expect(find(dirty, "cef-bytes")[0]!.detail).toContain('engine "system"');
  });

  test("with chromium configured the same bundle is what the check wants to see", () => {
    const config: NativeDesktopConfig = { webview: { engine: { [target]: "chromium" } } };
    const app = tempApp();
    stageBundle(app, true);
    expect(find(webviewChecks(config, app), "cef-bytes")[0]!.status).toBe("ok");

    const empty = tempApp();
    stageBundle(empty, false);
    expect(find(webviewChecks(config, empty), "cef-bytes")[0]!.status).toBe("error");
  });
});
