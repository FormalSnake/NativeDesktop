// Webview engine and scheme resolution: the decisions `nd dev`, `nd package`
// and `nd doctor` all make from the same config, and the values `nd dev` hands
// the host as ND_WEBVIEW_ENGINE and ND_CEF_SCHEMES.
import { describe, expect, test } from "bun:test";
import { engineTargetFor, type NativeDesktopConfig, resolveCefSchemes, resolveWebViewEngine } from "./config.ts";

const chromiumOnMac: NativeDesktopConfig = { webview: { engine: { mac: "chromium" } } };

describe("engineTargetFor", () => {
  test("maps the packaging platforms, and nothing else yet", () => {
    expect(engineTargetFor("darwin")).toBe("mac");
    expect(engineTargetFor("linux")).toBe("linux");
    expect(engineTargetFor("win32")).toBeUndefined();
  });
});

describe("resolveWebViewEngine", () => {
  test("defaults to system, per platform", () => {
    expect(resolveWebViewEngine({}, "mac", {})).toBe("system");
    expect(resolveWebViewEngine(chromiumOnMac, "mac", {})).toBe("chromium");
    expect(resolveWebViewEngine(chromiumOnMac, "linux", {})).toBe("system");
  });

  test("ND_WEBVIEW_ENGINE overrides the config in both directions", () => {
    expect(resolveWebViewEngine({}, "linux", { ND_WEBVIEW_ENGINE: "chromium" })).toBe("chromium");
    expect(resolveWebViewEngine(chromiumOnMac, "mac", { ND_WEBVIEW_ENGINE: "system" })).toBe("system");
  });

  test("an unknown engine names where it came from", () => {
    expect(() => resolveWebViewEngine({}, "mac", { ND_WEBVIEW_ENGINE: "webkit2" })).toThrow("ND_WEBVIEW_ENGINE");
    const bad = { webview: { engine: { linux: "blink" } } } as unknown as NativeDesktopConfig;
    expect(() => resolveWebViewEngine(bad, "linux", {})).toThrow("webview.engine.linux");
  });
});

describe("resolveCefSchemes", () => {
  const declared: NativeDesktopConfig = { webview: { cef: { schemes: ["nbext", "nbext"] } } };

  test("defaults to none, and deduplicates what the config declares", () => {
    expect(resolveCefSchemes({}, {})).toEqual([]);
    expect(resolveCefSchemes(declared, {})).toEqual(["nbext"]);
  });

  test("ND_CEF_SCHEMES overrides the config, comma separated", () => {
    expect(resolveCefSchemes(declared, { ND_CEF_SCHEMES: "one, two" })).toEqual(["one", "two"]);
    expect(resolveCefSchemes(declared, { ND_CEF_SCHEMES: "" })).toEqual([]);
  });

  test("a name that is not a scheme names where it came from", () => {
    expect(() => resolveCefSchemes({}, { ND_CEF_SCHEMES: "NBExt" })).toThrow("ND_CEF_SCHEMES");
    const bad = { webview: { cef: { schemes: ["nb ext"] } } };
    expect(() => resolveCefSchemes(bad, {})).toThrow("webview.cef.schemes");
  });
});
