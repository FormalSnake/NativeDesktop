// Resolution matrix for @nativedesktop/host: backend selection across
// platform × env × flag, and the per-backend binary path layout.
// Run with: bun test packages/host/

import { test, expect } from "bun:test";
import { resolve } from "node:path";
import { hostBinaryCandidates, hostPlatformKey, resolveBackend } from "./index.ts";

// --- backend selection: platform × env × flag ------------------------------

test("default backend follows the platform", () => {
  expect(resolveBackend({}, {}, "darwin")).toBe("appkit");
  expect(resolveBackend({}, {}, "linux")).toBe("gtk");
  expect(resolveBackend({}, {}, "win32")).toBe("gtk");
});

test("ND_BACKEND overrides the platform default", () => {
  expect(resolveBackend({}, { ND_BACKEND: "gtk" }, "darwin")).toBe("gtk");
  expect(resolveBackend({}, { ND_BACKEND: "appkit" }, "darwin")).toBe("appkit");
});

test("explicit backend option beats ND_BACKEND and the default", () => {
  expect(resolveBackend({ backend: "gtk" }, { ND_BACKEND: "appkit" }, "darwin")).toBe("gtk");
  expect(resolveBackend({ backend: "appkit" }, { ND_BACKEND: "gtk" }, "darwin")).toBe("appkit");
});

test("appkit is macOS-only — requesting it elsewhere errors", () => {
  expect(() => resolveBackend({ backend: "appkit" }, {}, "linux")).toThrow(/macOS-only/);
  expect(() => resolveBackend({}, { ND_BACKEND: "appkit" }, "linux")).toThrow(/macOS-only/);
});

test("an unknown backend errors", () => {
  expect(() => resolveBackend({ backend: "qt" as never }, {}, "linux")).toThrow(/unknown backend/);
  expect(() => resolveBackend({}, { ND_BACKEND: "qt" }, "linux")).toThrow(/unknown backend/);
});

// --- binary path layout: backend × platform × arch -------------------------

const PKG = resolve(import.meta.dir, "..");

test("gtk resolves nd-hello prebuilt + fresh zig-out artifact", () => {
  const c = hostBinaryCandidates("gtk", { platform: "linux", arch: "x64", packageDir: PKG });
  expect(c.prebuilt).toBe(resolve(PKG, "bin", "linux-x64", "nd-hello"));
  expect(c.fresh).toEqual([resolve(c.repoRoot, "zig-out", "bin", "nd-hello")]);
});

test("appkit resolves nd-shell prebuilt + fresh swift artifacts (release before debug)", () => {
  const c = hostBinaryCandidates("appkit", { platform: "darwin", arch: "arm64", packageDir: PKG });
  expect(c.prebuilt).toBe(resolve(PKG, "bin", "darwin-arm64", "nd-shell"));
  expect(c.fresh).toEqual([
    resolve(c.repoRoot, "swift", ".build", "release", "NDShell"),
    resolve(c.repoRoot, "swift", ".build", "debug", "NDShell"),
  ]);
});

test("windows gtk binary carries the .exe suffix", () => {
  const c = hostBinaryCandidates("gtk", { platform: "win32", arch: "x64", packageDir: PKG });
  expect(c.prebuilt).toBe(resolve(PKG, "bin", "windows-x64", "nd-hello.exe"));
});

test("repoRoot is two levels above the package", () => {
  const c = hostBinaryCandidates("gtk", { platform: "linux", arch: "x64", packageDir: PKG });
  expect(c.repoRoot).toBe(resolve(PKG, "..", ".."));
});

test("hostPlatformKey rejects unsupported platforms", () => {
  expect(hostPlatformKey("darwin", "arm64")).toBe("darwin-arm64");
  expect(() => hostPlatformKey("sunos", "arm64")).toThrow(/unsupported platform/);
});
