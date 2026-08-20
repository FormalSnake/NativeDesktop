// CEF acquisition and staging. The mac leg runs against the real cached dist
// when the machine has one (no download from a test) and against a synthetic
// framework otherwise; the linux dist is never fetched here, so its leg builds
// the dist tree it expects and asserts the plan over that.
import { describe, expect, test } from "bun:test";
import { existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { cefDistDir, isCefRoot } from "@nativedesktop/host/cef";
import {
  applyCefMacPlan,
  cefBundleAudit,
  type CefIndex,
  cefMacSignTargets,
  cefVersionFor,
  CEF_HELPER_BINARY_NAME,
  CEF_HELPER_SUFFIXES,
  ensureCefDist,
  fetchCefIndex,
  planCefLinux,
  planCefMac,
  resolveCefHelperBinary,
  selectCefBuild,
} from "./cef.ts";

const FRAMEWORK = "Chromium Embedded Framework.framework";
const PINNED = cefVersionFor(undefined);

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "nd-cef-pkg-"));
}

function macPlanInput(distRoot: string, contents: string, helperBinary: string) {
  return {
    distRoot,
    contents,
    appName: "Demo",
    appId: "com.example.demo",
    version: "1.2.3",
    minimumSystemVersion: "26.0",
    helperBinary,
  };
}

/** A CEF mac dist in the versioned framework layout, symlinks and all. The
 * shipped 151.3.23 dist is flat, but codesign only accepts a versioned bundle
 * when the links survive the copy, so the fixture keeps them. */
function synthesizeMacDist(root: string): string {
  const framework = join(root, "Release", FRAMEWORK);
  const versionA = join(framework, "Versions", "A");
  mkdirSync(join(versionA, "Libraries"), { recursive: true });
  mkdirSync(join(versionA, "Resources"), { recursive: true });
  writeFileSync(join(versionA, "Chromium Embedded Framework"), "mach-o");
  writeFileSync(join(versionA, "Libraries", "libEGL.dylib"), "mach-o");
  writeFileSync(join(versionA, "Libraries", "libGLESv2.dylib"), "mach-o");
  writeFileSync(join(versionA, "Resources", "Info.plist"), "<plist/>");
  symlinkSync("A", join(framework, "Versions", "Current"));
  symlinkSync("Versions/Current/Chromium Embedded Framework", join(framework, "Chromium Embedded Framework"));
  symlinkSync("Versions/Current/Libraries", join(framework, "Libraries"));
  symlinkSync("Versions/Current/Resources", join(framework, "Resources"));
  return root;
}

/** The linux dist's own file set, from cmake/cef_variables.cmake. Empty files:
 * the plan is about layout, and downloading 269 MB in a unit test is not. */
function synthesizeLinuxDist(root: string, locales: string[]): string {
  const release = join(root, "Release");
  const resources = join(root, "Resources");
  mkdirSync(release, { recursive: true });
  mkdirSync(join(resources, "locales"), { recursive: true });
  for (const name of [
    "libcef.so",
    "libEGL.so",
    "libGLESv2.so",
    "libvk_swiftshader.so",
    "libvulkan.so.1",
    "vk_swiftshader_icd.json",
    "v8_context_snapshot.bin",
    "chrome-sandbox",
  ]) writeFileSync(join(release, name), "");
  for (const name of ["icudtl.dat", "resources.pak", "chrome_100_percent.pak", "chrome_200_percent.pak"]) {
    writeFileSync(join(resources, name), "");
  }
  for (const locale of locales) writeFileSync(join(resources, "locales", `${locale}.pak`), "");
  return root;
}

const INDEX: CefIndex = {
  macosarm64: {
    versions: [
      {
        cef_version: "152.0.1+gaaaaaaa+chromium-152.0.0.1",
        channel: "stable",
        files: [{ type: "minimal", name: "cef_binary_152.0.1_macosarm64_minimal.tar.bz2", sha1: "newer", size: 1 }],
      },
      {
        cef_version: "151.3.23+gd211df0+chromium-151.0.7922.170",
        channel: "stable",
        files: [
          { type: "standard", name: "cef_binary_151.3.23+gd211df0+chromium-151.0.7922.170_macosarm64.tar.bz2", sha1: "std", size: 297930532 },
          { type: "minimal", name: "cef_binary_151.3.23+gd211df0+chromium-151.0.7922.170_macosarm64_minimal.tar.bz2", sha1: "94b409659401b34bf8616f9592ba5daeb9ee82d4", size: 130993494 },
        ],
      },
      {
        cef_version: "151.3.2+gc0ffee0+chromium-151.0.7922.100",
        channel: "stable",
        files: [{ type: "minimal", name: "cef_binary_151.3.2_macosarm64_minimal.tar.bz2", sha1: "older", size: 1 }],
      },
    ],
  },
};

describe("selectCefBuild", () => {
  test("picks the minimal dist for the pinned release and percent-encodes the URL", () => {
    const build = selectCefBuild(INDEX, "macosarm64", "151.3.23");
    expect(build.cefVersion).toBe("151.3.23+gd211df0+chromium-151.0.7922.170");
    expect(build.sha1).toBe("94b409659401b34bf8616f9592ba5daeb9ee82d4");
    expect(build.size).toBe(130993494);
    expect(build.url).toBe(
      "https://cef-builds.spotifycdn.com/cef_binary_151.3.23%2Bgd211df0%2Bchromium-151.0.7922.170_macosarm64_minimal.tar.bz2",
    );
  });

  test("the pin matches on the + boundary, so 151.3.2 is not 151.3.23", () => {
    expect(selectCefBuild(INDEX, "macosarm64", "151.3.2").cefVersion).toBe("151.3.2+gc0ffee0+chromium-151.0.7922.100");
  });

  test("names the newest build when the pin is not in the index", () => {
    expect(() => selectCefBuild(INDEX, "macosarm64", "9.9.9")).toThrow("152.0.1");
    expect(() => selectCefBuild(INDEX, "linux64", "151.3.23")).toThrow("no builds");
  });
});

describe("fetchCefIndex", () => {
  test("ND_CEF_INDEX reads a local copy instead of the network", async () => {
    const path = join(tempDir(), "index.json");
    writeFileSync(path, JSON.stringify(INDEX));
    expect(await fetchCefIndex({ ND_CEF_INDEX: path })).toEqual(INDEX);
  });
});

describe("ensureCefDist", () => {
  test("an already-extracted dist is returned without a download", async () => {
    const dist = cefDistDir(PINNED, "macosarm64");
    if (!isCefRoot(join(dist, "Release"), "macosarm64")) {
      console.error(`ND_TEST_SKIP no cached CEF ${PINNED} macosarm64 dist at ${dist}`);
      return;
    }
    expect(await ensureCefDist({ cefPlatform: "macosarm64", version: PINNED, offline: true })).toBe(dist);
  });

  test("offline with nothing cached names the path it wanted", async () => {
    const env = { ...process.env, XDG_CACHE_HOME: tempDir() };
    expect(ensureCefDist({ cefPlatform: "linux64", version: PINNED, env, offline: true }))
      .rejects.toThrow("downloads are disabled");
  });
});

describe("planCefMac", () => {
  test("stages the framework and the five helper bundles CEF requires", () => {
    const root = tempDir();
    const dist = synthesizeMacDist(join(root, "dist"));
    const contents = join(root, "Demo.app", "Contents");
    const plan = planCefMac(macPlanInput(dist, contents, join(root, CEF_HELPER_BINARY_NAME)));

    expect(plan.framework.to).toBe(join(contents, "Frameworks", FRAMEWORK));
    expect(plan.frameworkLibraries.map((p) => p.replace(`${plan.framework.to}/`, "")))
      .toEqual(["Libraries/libEGL.dylib", "Libraries/libGLESv2.dylib"]);
    expect(plan.helpers.map((h) => h.suffix)).toEqual([...CEF_HELPER_SUFFIXES]);
    expect(plan.helpers.map((h) => h.bundleId)).toEqual([
      "com.example.demo.helper",
      "com.example.demo.helper.alerts",
      "com.example.demo.helper.gpu",
      "com.example.demo.helper.plugin",
      "com.example.demo.helper.renderer",
    ]);
    expect(plan.helpers.map((h) => h.jit)).toEqual([false, false, true, false, true]);
    expect(plan.helpers[4]!.executablePath)
      .toBe(join(contents, "Frameworks", "Demo Helper (Renderer).app", "Contents", "MacOS", "Demo Helper (Renderer)"));
    expect(plan.helpers[0]!.plist).toContain("<key>CFBundleIdentifier</key><string>com.example.demo.helper</string>");
    expect(plan.helpers[0]!.plist).toContain("<key>LSUIElement</key><string>1</string>");
  });

  test("rejects a dist with no framework in it", () => {
    const root = tempDir();
    mkdirSync(join(root, "dist", "Release"), { recursive: true });
    expect(() => planCefMac(macPlanInput(join(root, "dist"), join(root, "x", "Contents"), "/nope")))
      .toThrow("has no Chromium Embedded Framework.framework");
  });

  test("the real cached dist plans the same way", () => {
    const dist = cefDistDir(PINNED, "macosarm64");
    if (!isCefRoot(join(dist, "Release"), "macosarm64")) {
      console.error(`ND_TEST_SKIP no cached CEF ${PINNED} macosarm64 dist at ${dist}`);
      return;
    }
    const root = tempDir();
    const plan = planCefMac(macPlanInput(dist, join(root, "Demo.app", "Contents"), join(root, CEF_HELPER_BINARY_NAME)));
    expect(plan.framework.from).toBe(join(dist, "Release", FRAMEWORK));
    expect(plan.frameworkLibraries.length).toBeGreaterThan(0);
    expect(plan.helpers).toHaveLength(5);
  });
});

describe("cefMacSignTargets", () => {
  test("signs inside-out: framework Mach-O, the framework, then each helper", () => {
    const root = tempDir();
    const dist = synthesizeMacDist(join(root, "dist"));
    const contents = join(root, "Demo.app", "Contents");
    const plan = planCefMac(macPlanInput(dist, contents, join(root, CEF_HELPER_BINARY_NAME)));
    const targets = cefMacSignTargets(plan).map((t) => ({ path: t.path.replace(`${contents}/Frameworks/`, ""), jit: t.jit }));

    expect(targets.slice(0, 3)).toEqual([
      { path: `${FRAMEWORK}/Libraries/libEGL.dylib`, jit: false },
      { path: `${FRAMEWORK}/Libraries/libGLESv2.dylib`, jit: false },
      { path: FRAMEWORK, jit: false },
    ]);
    // Each helper's executable precedes its bundle, and only renderer/GPU ask
    // for the JIT entitlements.
    expect(targets.slice(3)).toEqual([
      { path: "Demo Helper.app/Contents/MacOS/Demo Helper", jit: false },
      { path: "Demo Helper.app", jit: false },
      { path: "Demo Helper (Alerts).app/Contents/MacOS/Demo Helper (Alerts)", jit: false },
      { path: "Demo Helper (Alerts).app", jit: false },
      { path: "Demo Helper (GPU).app/Contents/MacOS/Demo Helper (GPU)", jit: true },
      { path: "Demo Helper (GPU).app", jit: true },
      { path: "Demo Helper (Plugin).app/Contents/MacOS/Demo Helper (Plugin)", jit: false },
      { path: "Demo Helper (Plugin).app", jit: false },
      { path: "Demo Helper (Renderer).app/Contents/MacOS/Demo Helper (Renderer)", jit: true },
      { path: "Demo Helper (Renderer).app", jit: true },
    ]);
  });
});

describe("applyCefMacPlan", () => {
  test("keeps the framework's symlinks as symlinks and writes the helper bundles", () => {
    const root = tempDir();
    const dist = synthesizeMacDist(join(root, "dist"));
    const contents = join(root, "Demo.app", "Contents");
    const helper = join(root, CEF_HELPER_BINARY_NAME);
    writeFileSync(helper, "helper");
    const plan = planCefMac(macPlanInput(dist, contents, helper));
    applyCefMacPlan(plan);

    expect(lstatSync(join(plan.framework.to, "Chromium Embedded Framework")).isSymbolicLink()).toBe(true);
    expect(lstatSync(join(plan.framework.to, "Versions", "Current")).isSymbolicLink()).toBe(true);
    expect(lstatSync(join(plan.framework.to, "Versions", "A", "Chromium Embedded Framework")).isFile()).toBe(true);
    for (const h of plan.helpers) {
      expect(readFileSync(h.executablePath, "utf8")).toBe("helper");
      expect(existsSync(join(h.appPath, "Contents", "Info.plist"))).toBe(true);
    }
  });

  test("names the missing helper binary instead of producing a broken bundle", () => {
    const root = tempDir();
    const dist = synthesizeMacDist(join(root, "dist"));
    const plan = planCefMac(macPlanInput(dist, join(root, "Demo.app", "Contents"), join(root, CEF_HELPER_BINARY_NAME)));
    expect(() => applyCefMacPlan(plan)).toThrow(CEF_HELPER_BINARY_NAME);
  });
});

describe("resolveCefHelperBinary", () => {
  test("ND_CEF_HELPER wins, else the binary sits beside the host", () => {
    expect(resolveCefHelperBinary("/b/nd-shell", { ND_CEF_HELPER: "/x/helper" })).toBe("/x/helper");
    expect(resolveCefHelperBinary("/b/nd-shell", {})).toBe(`/b/${CEF_HELPER_BINARY_NAME}`);
  });
});

describe("planCefLinux", () => {
  test("stages CEF under lib/cef, strips only libcef.so, and defaults to en-US", () => {
    const root = tempDir();
    const dist = synthesizeLinuxDist(join(root, "dist"), ["en-US", "de", "fr"]);
    const plan = planCefLinux({ distRoot: dist, appDir: join(root, "AppDir") });
    const staged = plan.files.map((f) => f.to.replace(`${plan.root}/`, ""));

    expect(plan.root).toBe(join(root, "AppDir", "lib", "cef"));
    expect(plan.missing).toEqual([]);
    expect(staged).toEqual([
      "libcef.so",
      "libEGL.so",
      "libGLESv2.so",
      "libvk_swiftshader.so",
      "libvulkan.so.1",
      "vk_swiftshader_icd.json",
      "v8_context_snapshot.bin",
      "chrome-sandbox",
      "icudtl.dat",
      "resources.pak",
      "chrome_100_percent.pak",
      "chrome_200_percent.pak",
    ]);
    expect(plan.files.filter((f) => f.strip).map((f) => f.to.replace(`${plan.root}/`, ""))).toEqual(["libcef.so"]);
    expect(plan.locales.map((f) => f.to.replace(`${plan.root}/`, ""))).toEqual(["locales/en-US.pak"]);
  });

  test("the config's locale list is the whole locales/ directory", () => {
    const root = tempDir();
    const dist = synthesizeLinuxDist(join(root, "dist"), ["en-US", "de", "nl"]);
    const plan = planCefLinux({ distRoot: dist, appDir: join(root, "AppDir"), locales: ["de", "nl"] });
    expect(plan.locales.map((f) => f.to.replace(`${plan.root}/`, ""))).toEqual(["locales/de.pak", "locales/nl.pak"]);
    expect(plan.missing).toEqual([]);
  });

  test("a locale the dist does not carry is reported, not silently dropped", () => {
    const root = tempDir();
    const dist = synthesizeLinuxDist(join(root, "dist"), ["en-US"]);
    const plan = planCefLinux({ distRoot: dist, appDir: join(root, "AppDir"), locales: ["en-US", "kl"] });
    expect(plan.missing).toEqual(["Resources/locales/kl.pak"]);
  });
});

describe("cefBundleAudit", () => {
  test("an engine=system bundle reports nothing", () => {
    const root = tempDir();
    mkdirSync(join(root, "Demo.app", "Contents", "Frameworks"), { recursive: true });
    writeFileSync(join(root, "Demo.app", "Contents", "MacOS"), "");
    expect(cefBundleAudit(join(root, "Demo.app"))).toEqual([]);
  });

  test("a staged framework and a staged libcef.so are both found", () => {
    const root = tempDir();
    const frameworks = join(root, "Demo.app", "Contents", "Frameworks");
    mkdirSync(join(frameworks, FRAMEWORK), { recursive: true });
    expect(cefBundleAudit(join(root, "Demo.app"))).toEqual([`Contents/Frameworks/${FRAMEWORK}`]);

    const appdir = join(root, "AppDir", "lib", "cef");
    mkdirSync(appdir, { recursive: true });
    writeFileSync(join(appdir, "libcef.so"), "");
    writeFileSync(join(appdir, "icudtl.dat"), "");
    expect(cefBundleAudit(join(root, "AppDir"))).toEqual(["lib/cef/icudtl.dat", "lib/cef/libcef.so"]);
  });
});
