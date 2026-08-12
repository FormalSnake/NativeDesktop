// Resolution matrix for @nativedesktop/host: backend selection across
// platform × env × flag, and the per-backend platform-package + fresh-artifact
// layout. Run with: bun test packages/host/
import { test, expect } from "bun:test";
import { resolve } from "node:path";
import { hostBinaryCandidates, hostPackageName, hostPlatformKey, resolveBackend, resolveHostBinary } from "./index.ts";

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

test("appkit is macOS-only: requesting it elsewhere errors", () => {
  expect(() => resolveBackend({ backend: "appkit" }, {}, "linux")).toThrow(/macOS-only/);
  expect(() => resolveBackend({}, { ND_BACKEND: "appkit" }, "linux")).toThrow(/macOS-only/);
});

test("an unknown backend errors", () => {
  expect(() => resolveBackend({ backend: "qt" as never }, {}, "linux")).toThrow(/unknown backend/);
  expect(() => resolveBackend({}, { ND_BACKEND: "qt" }, "linux")).toThrow(/unknown backend/);
});

// --- platform packages: backend × platform × arch --------------------------

test("each shipping target maps to its platform package", () => {
  expect(hostPackageName("appkit", "darwin-arm64")).toBe("@nativedesktop/host-darwin-arm64");
  expect(hostPackageName("gtk", "linux-x64")).toBe("@nativedesktop/host-linux-x64");
});

test("combinations without a prebuilt have no package", () => {
  expect(hostPackageName("gtk", "darwin-arm64")).toBeUndefined();
  expect(hostPackageName("appkit", "linux-x64")).toBeUndefined();
  expect(hostPackageName("gtk", "windows-x64")).toBeUndefined();
});

// --- candidates: package name, binary name, fresh artifacts ----------------

const PKG = resolve(import.meta.dir, "..");

test("gtk resolves the linux platform package + fresh zig-out artifact", () => {
  const c = hostBinaryCandidates("gtk", { platform: "linux", arch: "x64", packageDir: PKG });
  expect(c.packageName).toBe("@nativedesktop/host-linux-x64");
  expect(c.binaryName).toBe("nd-hello");
  expect(c.fresh).toEqual([resolve(c.repoRoot, "zig-out", "bin", "nd-hello")]);
});

test("appkit resolves the darwin platform package + fresh swift artifacts (release before debug)", () => {
  const c = hostBinaryCandidates("appkit", { platform: "darwin", arch: "arm64", packageDir: PKG });
  expect(c.packageName).toBe("@nativedesktop/host-darwin-arm64");
  expect(c.binaryName).toBe("nd-shell");
  expect(c.fresh).toEqual([
    resolve(c.repoRoot, "swift", ".build", "release", "NDShell"),
    resolve(c.repoRoot, "swift", ".build", "debug", "NDShell"),
  ]);
});

test("windows gtk binary carries the .exe suffix and has no package", () => {
  const c = hostBinaryCandidates("gtk", { platform: "win32", arch: "x64", packageDir: PKG });
  expect(c.binaryName).toBe("nd-hello.exe");
  expect(c.packageName).toBeUndefined();
});

test("repoRoot is two levels above the package", () => {
  const c = hostBinaryCandidates("gtk", { platform: "linux", arch: "x64", packageDir: PKG });
  expect(c.repoRoot).toBe(resolve(PKG, "..", ".."));
});

test("hostPlatformKey rejects unsupported platforms", () => {
  expect(hostPlatformKey("darwin", "arm64")).toBe("darwin-arm64");
  expect(() => hostPlatformKey("sunos", "arm64")).toThrow(/unsupported platform/);
});

// --- resolveHostBinary error path: gtk on macOS, installed (not a checkout) ---

test("gtk on darwin outside a source checkout states the three real options", async () => {
  // packageDir has no build.zig/swift two levels up, so isSourceCheckout is
  // false -- the shape of an installed copy inside some other project's
  // node_modules (packageName is already undefined for gtk+darwin regardless
  // of packageDir, since PLATFORM_PACKAGES has no entry for it).
  const err = await resolveHostBinary({ backend: "gtk", platform: "darwin", arch: "arm64", packageDir: "/tmp/not-a-checkout/node_modules/@nativedesktop/host" }).catch(
    (e: Error) => e,
  );
  expect(err).toBeInstanceOf(Error);
  const message = (err as Error).message;
  expect(message).toContain("by design");
  expect(message).toContain("source checkout");
  expect(message).toContain("explicit binary path");
});
