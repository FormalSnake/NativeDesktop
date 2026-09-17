// Webview engine, scheme and style resolution: the decisions `nd dev`,
// `nd package` and `nd doctor` all make from the same config, and the values
// `nd dev` hands the host as ND_WEBVIEW_ENGINE, ND_CEF_SCHEMES and ND_CEF_STYLE.
import { describe, expect, test } from "bun:test";
import {
  engineTargetFor,
  type NativeDesktopConfig,
  resolveCefSchemes,
  resolveCefStyle,
  resolveWebViewEngine,
} from "./config.ts";

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

describe("resolveCefStyle", () => {
  const chrome: NativeDesktopConfig = { webview: { cef: { style: "chrome" } } };

  test("defaults to alloy", () => {
    expect(resolveCefStyle({}, {})).toBe("alloy");
    expect(resolveCefStyle(chrome, {})).toBe("chrome");
  });

  test("ND_CEF_STYLE overrides the config in both directions", () => {
    expect(resolveCefStyle({}, { ND_CEF_STYLE: "chrome" })).toBe("chrome");
    expect(resolveCefStyle(chrome, { ND_CEF_STYLE: "alloy" })).toBe("alloy");
  });

  test("an unknown style names where it came from", () => {
    expect(() => resolveCefStyle({}, { ND_CEF_STYLE: "views" })).toThrow("ND_CEF_STYLE");
    const bad = { webview: { cef: { style: "blink" } } } as unknown as NativeDesktopConfig;
    expect(() => resolveCefStyle(bad, {})).toThrow("webview.cef.style");
  });
});
