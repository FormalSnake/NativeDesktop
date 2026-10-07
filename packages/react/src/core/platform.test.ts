// setBackend/setHostManifest state must survive a `bun --hot` re-eval, which
// re-runs the whole module graph while globalThis persists (render() skips
// the connect block on a re-eval and never re-installs). A query-string
// dynamic import forces a second, fresh evaluation of the module in this
// process — the same shape as a hot re-eval.
import { test, expect, afterEach } from "bun:test";
import { Platform, setBackend, setHostManifest } from "./platform.ts";

afterEach(() => {
  globalThis.__nd_platform = undefined;
});

test("select falls back to default before the handshake", () => {
  expect(Platform.backend).toBe("unknown");
  expect(Platform.select({ gtk: "G", default: "D" })).toBe("D");
});

test("backend + manifest survive a fresh module evaluation", async () => {
  setBackend("gtk");
  setHostManifest(new Set(["window"]), new Set(["window.present"]));
  const fresh = (await import("./platform.ts?reeval=1")) as typeof import("./platform.ts");
  expect(fresh.Platform.backend).toBe("gtk");
  expect(fresh.Platform.select({ gtk: "G", appkit: "A" })).toBe("G");
  expect(fresh.hasWidget("window")).toBe(true);
  expect(fresh.hasWidget("webview")).toBe(false);
  expect(fresh.hasCommand("window", "present")).toBe(true);
  expect(fresh.hasCommand("webview", "goBack")).toBe(false);
});
