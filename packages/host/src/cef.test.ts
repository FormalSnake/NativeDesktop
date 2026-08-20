// CEF resolution order. The engine backends read the same three places in the
// same sequence natively, so the order here is a contract, not a preference.
import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  bundleCefDir,
  cefCacheRoot,
  cefDistDir,
  cefLoadableName,
  cefPlatformKey,
  cefRootCandidates,
  isCefRoot,
  resolveCefRoot,
} from "./cef.ts";

describe("cefPlatformKey", () => {
  test("maps os/arch onto the names cef-builds.spotifycdn.com uses", () => {
    expect(cefPlatformKey("darwin", "arm64")).toBe("macosarm64");
    expect(cefPlatformKey("darwin", "x64")).toBe("macosx64");
    expect(cefPlatformKey("linux", "x64")).toBe("linux64");
    expect(cefPlatformKey("linux", "arm64")).toBe("linuxarm64");
  });

  test("rejects a platform CEF has no build for", () => {
    expect(() => cefPlatformKey("win32", "arm64")).toThrow("no CEF build");
  });
});

describe("cefCacheRoot", () => {
  test("honors XDG_CACHE_HOME, else ~/.cache", () => {
    expect(cefCacheRoot({ XDG_CACHE_HOME: "/x", HOME: "/home/k" })).toBe("/x/nativedesktop/cef");
    expect(cefCacheRoot({ HOME: "/home/k" })).toBe("/home/k/.cache/nativedesktop/cef");
  });

  test("the dist dir is keyed version-platform", () => {
    expect(cefDistDir("151.3.23", "macosarm64", { HOME: "/home/k" }))
      .toBe("/home/k/.cache/nativedesktop/cef/151.3.23-macosarm64");
  });
});

describe("cefRootCandidates", () => {
  test("ND_CEF_ROOT, then the bundle, then the dev cache", () => {
    const candidates = cefRootCandidates({
      cefPlatform: "macosarm64",
      version: "151.3.23",
      bundleRoot: "/apps/Demo.app",
      env: { ND_CEF_ROOT: "/opt/cef", HOME: "/home/k" },
    });
    expect(candidates).toEqual([
      "/opt/cef",
      "/apps/Demo.app/Contents/Frameworks",
      "/home/k/.cache/nativedesktop/cef/151.3.23-macosarm64/Release",
    ]);
  });

  test("without ND_CEF_ROOT or a bundle only the cache remains", () => {
    expect(cefRootCandidates({ cefPlatform: "linux64", version: "151.3.23", env: { HOME: "/home/k" } }))
      .toEqual(["/home/k/.cache/nativedesktop/cef/151.3.23-linux64/Release"]);
  });

  test("a linux bundle keeps CEF under lib/cef", () => {
    expect(bundleCefDir("/build/AppDir", "linux64")).toBe("/build/AppDir/lib/cef");
  });
});

describe("resolveCefRoot", () => {
  test("skips candidates that hold no loadable and takes the first that does", () => {
    const home = mkdtempSync(join(tmpdir(), "nd-cef-"));
    const empty = join(home, "empty");
    mkdirSync(empty, { recursive: true });
    const cache = join(home, ".cache", "nativedesktop", "cef", "151.3.23-linux64", "Release");
    mkdirSync(cache, { recursive: true });
    writeFileSync(join(cache, cefLoadableName("linux64")), "");

    const env = { HOME: home, ND_CEF_ROOT: empty };
    expect(isCefRoot(empty, "linux64")).toBe(false);
    expect(resolveCefRoot({ cefPlatform: "linux64", version: "151.3.23", env })).toBe(cache);
  });

  test("undefined when nothing on disk answers", () => {
    const home = mkdtempSync(join(tmpdir(), "nd-cef-"));
    expect(resolveCefRoot({ cefPlatform: "linux64", version: "151.3.23", env: { HOME: home } })).toBeUndefined();
  });
});
