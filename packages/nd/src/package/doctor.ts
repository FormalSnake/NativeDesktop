// `nd doctor`: checks the current directory's packaging readiness and the
// host toolchain. Warnings are advisory; only real gaps (a config that names
// missing files, an unresolvable host binary) exit non-zero.
import { existsSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { hostBinaryCandidates, prebuiltHostBinary, resolveBackend } from "@nativedesktop/host";
import { cefDistDir, cefPlatformKey, resolveCefRoot } from "@nativedesktop/host/cef";
import { engineTargetFor, loadConfig, type NativeDesktopConfig, resolveWebViewEngine, type WebViewEngine } from "../config.ts";
import { cefBundleAudit, cefVersionFor } from "./cef.ts";
import { DEFAULT_ENTRY } from "./payload.ts";

export type CheckStatus = "ok" | "warn" | "error";

export interface Check {
  name: string;
  status: CheckStatus;
  detail: string;
}

function which(tool: string): boolean {
  return Bun.which(tool) !== null;
}

/** Every mac icon source needs a different converter, so the check follows the
 * same resolution order `installMacIcon` uses. */
function macIconChecks(config: NativeDesktopConfig): Check[] {
  const icon = typeof config.app?.icon === "string" ? { source: config.app.icon } : config.app?.icon;
  if (!icon) return [];
  const source = icon.layered ? ".icon" : icon.macos ?? icon.source;
  if (!source) return [];
  if (source.endsWith(".icon")) {
    return [which("xcrun")
      ? { name: "icon-tools", status: "ok", detail: "layered icon configured, actool available" }
      : { name: "icon-tools", status: "error", detail: "layered icon configured but Xcode's actool is missing" }];
  }
  if (source.endsWith(".icns")) return [];
  if (source.endsWith(".iconset")) {
    return which("iconutil") ? [] : [{ name: "icon-tools", status: "error", detail: "iconset icon configured but iconutil is missing" }];
  }
  if (source.endsWith(".svg") && !["rsvg-convert", "magick", "convert", "qlmanage"].some(which)) {
    return [{ name: "icon-tools", status: "error", detail: "SVG icon configured but no rasterizer (librsvg, ImageMagick, or qlmanage) is available" }];
  }
  if (!(which("sips") && which("iconutil"))) {
    return [{ name: "icon-tools", status: "error", detail: `${source} icon configured but sips/iconutil are missing` }];
  }
  return [];
}

/** Bundles a previous `nd package` run left in the output dir, which is what the
 * zero-CEF-bytes claim can be checked against. */
export function builtBundles(config: NativeDesktopConfig, cwd: string): string[] {
  const outDir = resolve(cwd, config.package?.workspaceRoot ?? ".", config.package?.outDir ?? "dist");
  const bundles: string[] = [];
  const macDir = join(outDir, "mac");
  if (existsSync(macDir)) {
    for (const entry of readdirSync(macDir)) if (entry.endsWith(".app")) bundles.push(join(macDir, entry));
  }
  const appDir = join(outDir, "linux", "AppDir");
  if (existsSync(appDir)) bundles.push(appDir);
  return bundles;
}

/**
 * The engine the current platform resolves to, whether CEF can be found when it
 * asks for one, and whether the last packaged bundle matches: an engine="system"
 * bundle carrying Chromium bytes breaks the framework's central claim about the
 * default path, so it fails rather than warns.
 */
export function webviewChecks(config: NativeDesktopConfig, cwd: string): Check[] {
  const target = engineTargetFor();
  if (!target) return [];
  let engine: WebViewEngine;
  try {
    engine = resolveWebViewEngine(config, target);
  } catch (err) {
    return [{ name: "webview", status: "error", detail: String(err) }];
  }
  const checks: Check[] = [{
    name: "webview",
    status: "ok",
    detail: `engine=${engine} (${target})${process.env.ND_WEBVIEW_ENGINE ? " from ND_WEBVIEW_ENGINE" : ""}`,
  }];

  const version = cefVersionFor(config.webview?.cef);
  if (engine === "chromium") {
    try {
      const cefPlatform = cefPlatformKey();
      const root = resolveCefRoot({ cefPlatform, version });
      checks.push(root
        ? { name: "cef", status: "ok", detail: `CEF ${version} at ${root}` }
        : {
          name: "cef",
          status: "error",
          detail: `engine "chromium" but no CEF ${version} dist found (ND_CEF_ROOT, the app bundle, or ${cefDistDir(version, cefPlatform)}); \`nd package\` downloads it`,
        });
    } catch (err) {
      checks.push({ name: "cef", status: "error", detail: String(err) });
    }
    if (target === "linux") {
      checks.push({
        name: "cef",
        status: "warn",
        detail: "chrome-sandbox ships with the dist's own mode; the setuid sandbox wants root:root 4755 at install time (the userns sandbox is the automatic fallback, never --no-sandbox)",
      });
    }
  }

  for (const bundle of builtBundles(config, cwd)) {
    const found = cefBundleAudit(bundle);
    if (engine === "chromium") {
      checks.push(found.length
        ? { name: "cef-bytes", status: "ok", detail: `${bundle}: ${found.length} CEF artifact(s) staged` }
        : { name: "cef-bytes", status: "error", detail: `${bundle} carries no CEF payload but engine is "chromium" (repackage)` });
    } else {
      checks.push(found.length
        ? { name: "cef-bytes", status: "error", detail: `${bundle} ships CEF with engine "system": ${found.slice(0, 3).join(", ")}` }
        : { name: "cef-bytes", status: "ok", detail: `${bundle}: no CEF bytes` });
    }
  }
  return checks;
}

export async function collectChecks(cwd: string): Promise<Check[]> {
  const checks: Check[] = [];
  const configPath = resolve(cwd, "nativedesktop.config.ts");
  let config: NativeDesktopConfig = {};
  if (!existsSync(configPath)) {
    checks.push({ name: "config", status: "warn", detail: "no nativedesktop.config.ts here (defaults apply)" });
  } else {
    try {
      config = await loadConfig(cwd);
      checks.push({ name: "config", status: "ok", detail: "nativedesktop.config.ts loads" });
    } catch (err) {
      checks.push({ name: "config", status: "error", detail: `nativedesktop.config.ts failed to load: ${err}` });
      return checks;
    }
  }

  const hasConfig = existsSync(configPath);
  if (config.app?.id) {
    checks.push({ name: "app.id", status: "ok", detail: config.app.id });
  } else {
    // Icons, mime registration, and update manifests all key off app.id; a
    // configured app without one is an error, a bare directory just a warning.
    const status: CheckStatus = hasConfig && (config.app || config.package) ? "error" : "warn";
    checks.push({ name: "app.id", status, detail: "app.id not set (required for icons, file associations, and updates)" });
  }

  const entry = config.package?.entry ?? DEFAULT_ENTRY;
  if (existsSync(resolve(cwd, entry))) {
    checks.push({ name: "entry", status: "ok", detail: entry });
  } else {
    checks.push({
      name: "entry",
      status: hasConfig ? "error" : "warn",
      detail: `entry ${entry} not found (set package.entry)`,
    });
  }

  checks.push(which("bun")
    ? { name: "bun", status: "ok", detail: Bun.which("bun")! }
    : { name: "bun", status: "error", detail: "bun not found on PATH" });

  try {
    const backend = resolveBackend(process.platform === "darwin" ? { backend: config.package?.mac?.backend ?? "appkit" } : { backend: "gtk" });
    const { fresh } = hostBinaryCandidates(backend);
    // ND_HOST_BINARY is what `nd dev` will actually run, so report that rather
    // than the prebuilt it overrides.
    const explicit = process.env.ND_HOST_BINARY;
    const found = explicit || prebuiltHostBinary(backend) || fresh.find(existsSync);
    if (explicit && !existsSync(explicit)) {
      checks.push({ name: "host", status: "error", detail: `ND_HOST_BINARY points at ${explicit}, which does not exist` });
    } else {
      checks.push(found
        ? { name: "host", status: "ok", detail: explicit ? `${found} (ND_HOST_BINARY)` : found }
        : { name: "host", status: "warn", detail: `no ${backend} host binary built yet (nd dev / nd package builds it on first run in a source checkout)` });
    }
  } catch (err) {
    checks.push({ name: "host", status: "error", detail: String(err) });
  }

  if (process.platform === "darwin") {
    checks.push(which("codesign")
      ? { name: "codesign", status: "ok", detail: "codesign available" }
      : { name: "codesign", status: "error", detail: "codesign not found (install the Xcode command line tools)" });
    checks.push(...macIconChecks(config));
  } else if (process.platform === "linux") {
    checks.push(which("appimagetool") || which("mksquashfs")
      ? { name: "appimage", status: "ok", detail: which("appimagetool") ? "appimagetool available" : "mksquashfs fallback available" }
      : { name: "appimage", status: "warn", detail: "neither appimagetool nor mksquashfs on PATH (only the raw AppDir can be produced)" });
  }

  checks.push(...webviewChecks(config, cwd));

  if (config.package?.updates) {
    checks.push(which("minisign")
      ? { name: "updates", status: "ok", detail: "updates configured, minisign available" }
      : { name: "updates", status: "error", detail: "updates configured but minisign is not on PATH" });
  } else {
    checks.push({ name: "updates", status: "warn", detail: "package.updates not configured: the shipped app has no updater" });
  }

  return checks;
}

const GLYPH: Record<CheckStatus, string> = { ok: "ok  ", warn: "warn", error: "FAIL" };

/** Runs the check table against `cwd`, prints it (or JSON), returns the exit code. */
export async function runDoctor(cwd: string, json: boolean): Promise<number> {
  const checks = await collectChecks(cwd);
  if (json) {
    console.log(JSON.stringify(checks, null, 2));
  } else {
    for (const check of checks) console.log(`${GLYPH[check.status]}  ${check.name.padEnd(12)} ${check.detail}`);
  }
  return checks.some((check) => check.status === "error") ? 1 : 0;
}
