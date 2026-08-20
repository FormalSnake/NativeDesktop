// Webview engine resolution: the one decision `nd dev`, `nd package` and
// `nd doctor` all make from the same config, and the value `nd dev` hands the
// host as ND_WEBVIEW_ENGINE.
import { describe, expect, test } from "bun:test";
import { engineTargetFor, type NativeDesktopConfig, resolveWebViewEngine } from "./config.ts";

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
