// Pure-surface packaging units: identity resolution and the string
// transforms stamped into Info.plist / .desktop / mime XML / AppRun. No
// filesystem writes; resolveIdentity reads a fixture package.json.
import { describe, expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { AppIdentity } from "../config.ts";
import {
  buildDesktopEntry,
  buildInfoPlist,
  buildMimeInfoXml,
  injectDesktopFile,
  injectInfoPlist,
  resolveIdentity,
  slugify,
  type ResolvedIdentity,
} from "./identity.ts";
import { appRunTemplate, desktopEntryTemplate, infoPlistSkeleton } from "./templates.ts";

const IDENTITY: ResolvedIdentity = {
  id: "com.nativedesktop.gallery",
  name: "Gallery",
  displayName: "NativeDesktop Gallery",
  slug: "gallery",
  version: "0.9.0",
  categories: ["Utility"],
};

const INFO_PLIST = infoPlistSkeleton({
  name: "Gallery",
  displayName: "NativeDesktop Gallery",
  id: "com.nativedesktop.gallery",
  executable: "Gallery",
  version: "0.9.0",
  minimumSystemVersion: "26.0",
});

const DESKTOP_ENTRY = desktopEntryTemplate({
  displayName: "NativeDesktop Gallery",
  slug: "gallery",
  categories: ["Utility"],
});

describe("injectInfoPlist", () => {
  test("stamps CFBundleIdentifier and leaves the product name in place", () => {
    const out = injectInfoPlist(INFO_PLIST, { id: "es.canarycoders.canary" });
    expect(out).toContain("<key>CFBundleIdentifier</key><string>es.canarycoders.canary</string>");
    expect(out).toContain("<key>CFBundleName</key><string>Gallery</string>");
  });

  test("passes the plist through untouched when no app identity is configured", () => {
    expect(injectInfoPlist(INFO_PLIST, undefined)).toBe(INFO_PLIST);
  });

  test("adds CFBundleDocumentTypes and CFBundleURLTypes for configured associations/schemes", () => {
    const identity: AppIdentity = {
      id: "es.canarycoders.canary",
      fileAssociations: [{ ext: "canaryrun", name: "Canary Run", mimeType: "application/x-canary-run" }],
      urlSchemes: [{ scheme: "canary" }],
    };
    const out = injectInfoPlist(INFO_PLIST, identity);
    expect(out).toContain("<key>CFBundleDocumentTypes</key>");
    expect(out).toContain("<string>canaryrun</string>");
    expect(out).toContain("<key>CFBundleURLTypes</key>");
    expect(out).toContain("<string>canary</string>");
  });
});

describe("injectDesktopFile", () => {
  test("leaves a valid .desktop entry untouched when there's nothing to declare", () => {
    const out = injectDesktopFile(DESKTOP_ENTRY, { id: "com.nativedesktop.gallery" });
    expect(out).toBe(DESKTOP_ENTRY);
    expect(out).toContain("Name=NativeDesktop Gallery");
    expect(out).toContain("Exec=AppRun");
  });

  test("adds a MimeType= line and %U on Exec for configured file associations", () => {
    const identity: AppIdentity = {
      fileAssociations: [{ ext: "canaryrun", mimeType: "application/x-canary-run" }],
    };
    const out = injectDesktopFile(DESKTOP_ENTRY, identity);
    expect(out).toContain("MimeType=application/x-canary-run;");
    expect(out).toContain("Exec=AppRun %U");
  });
});

describe("buildMimeInfoXml", () => {
  test("returns null with no mimeType'd file associations", () => {
    expect(buildMimeInfoXml({ id: "com.nativedesktop.gallery" })).toBeNull();
  });

  test("declares each mimeType'd file association", () => {
    const xml = buildMimeInfoXml({
      fileAssociations: [{ ext: "canaryrun", name: "Canary Run", mimeType: "application/x-canary-run" }],
    });
    expect(xml).toContain('<mime-type type="application/x-canary-run">');
    expect(xml).toContain('<glob pattern="*.canaryrun"/>');
  });
});

describe("slugify", () => {
  test("lowercases and dashes the product name", () => {
    expect(slugify("Gallery")).toBe("gallery");
    expect(slugify("My App 2")).toBe("my-app-2");
  });

  test("rejects a name with no usable characters", () => {
    expect(() => slugify("!!!")).toThrow("empty slug");
  });
});

describe("resolveIdentity", () => {
  const dir = mkdtempSync(join(tmpdir(), "nd-identity-"));
  writeFileSync(join(dir, "package.json"), JSON.stringify({ name: "fixture-app", version: "1.2.3" }));

  test("defaults name/version from package.json", () => {
    const identity = resolveIdentity({}, dir);
    expect(identity.name).toBe("fixture-app");
    expect(identity.displayName).toBe("fixture-app");
    expect(identity.slug).toBe("fixture-app");
    expect(identity.version).toBe("1.2.3");
    expect(identity.categories).toEqual(["Utility"]);
  });

  test("config app fields win over package.json; --version wins over both", () => {
    const identity = resolveIdentity({ app: { name: "Fixture", version: "2.0.0" } }, dir, { version: "3.0.0" });
    expect(identity.name).toBe("Fixture");
    expect(identity.slug).toBe("fixture");
    expect(identity.version).toBe("3.0.0");
  });

  test("string icon becomes AppIcon.source", () => {
    const identity = resolveIdentity({ app: { icon: "assets/icon.png" } }, dir);
    expect(identity.icon).toEqual({ source: "assets/icon.png" });
  });
});

describe("buildInfoPlist", () => {
  test("generates the skeleton with identity fields and extraPlist entries", () => {
    const xml = buildInfoPlist(IDENTITY, { extraPlist: { LSUIElement: true, NSSupportsAutomaticTermination: false } });
    expect(xml).toContain("<key>CFBundleExecutable</key><string>Gallery</string>");
    expect(xml).toContain("<key>CFBundleIdentifier</key><string>com.nativedesktop.gallery</string>");
    expect(xml).toContain("<key>LSUIElement</key><true/>");
    expect(xml).toContain("<key>NSSupportsAutomaticTermination</key><false/>");
  });

  test("stamps executable and version into a custom Info.plist file", () => {
    const custom = infoPlistSkeleton({
      name: "Old",
      displayName: "Old",
      id: "com.old",
      executable: "NDShell",
      version: "0.0.1",
      minimumSystemVersion: "26.0",
    });
    const xml = buildInfoPlist(IDENTITY, undefined, custom);
    expect(xml).toContain("<key>CFBundleExecutable</key><string>Gallery</string>");
    expect(xml).toContain("<key>CFBundleShortVersionString</key><string>0.9.0</string>");
    expect(xml).toContain("<key>CFBundleIdentifier</key><string>com.nativedesktop.gallery</string>");
  });
});

describe("buildDesktopEntry / appRunTemplate", () => {
  test("desktop entry carries StartupWMClass and desktopEntry overrides", () => {
    const out = buildDesktopEntry(IDENTITY, { desktopEntry: { Comment: "Widget gallery" } });
    expect(out).toContain("StartupWMClass=com.nativedesktop.gallery");
    expect(out).toContain("Comment=Widget gallery");
    expect(out).toContain("Icon=gallery");
  });

  test("AppRun bakes entry, cwd, app id, and plugin paths", () => {
    const script = appRunTemplate({
      entry: "examples/gallery/main.tsx",
      cwd: "examples/gallery",
      slug: "gallery",
      appId: "com.nativedesktop.gallery",
      pluginPaths: ["native/libdemo.so"],
    });
    expect(script).toContain('export ND_SCRIPT="$HERE/app/examples/gallery/main.tsx"');
    expect(script).toContain('cd "$HERE/app/examples/gallery"');
    expect(script).toContain('export ND_APP_ID="com.nativedesktop.gallery"');
    expect(script).toContain('export ND_PLUGIN_PATHS="$HERE/app/native/libdemo.so"');
    expect(script).toContain('exec "$HERE/usr/bin/gallery" "$@"');
  });
});
