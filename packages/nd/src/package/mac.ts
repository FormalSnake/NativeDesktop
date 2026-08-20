// macOS packaging: assembles <Name>.app around the resolved host binary +
// bundled Bun + the app payload, deep-signs inside-out (ad-hoc by default;
// Developer-ID via config/APPLE_SIGN_IDENTITY) with the hardened runtime +
// allow-jit entitlements, and gates notarization on Apple credential presence.
import { $ } from "bun";
import { chmodSync, cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { resolveHostBinary } from "@nativedesktop/host";
import { cefPlatformKey } from "@nativedesktop/host/cef";
import { type NativeDesktopConfig, resolveCefSchemes, resolveWebViewEngine } from "../config.ts";
import {
  applyCefMacPlan,
  type CefMacPlan,
  cefMacSignTargets,
  cefVersionFor,
  ensureCefDist,
  ensureCefHelperBinary,
  planCefMac,
  writeCefHelperEntitlements,
} from "./cef.ts";
import { installMacIcon } from "./icons.ts";
import { buildInfoPlist, type ResolvedIdentity } from "./identity.ts";
import { assemblePayload } from "./payload.ts";
import { ENTITLEMENTS_PLIST } from "./templates.ts";
import { maybePublishUpdate } from "./updates.ts";
import type { PackageOptions, PackageResult } from "./index.ts";

/** Downloads (or reuses) the pinned CEF dist and stages the framework plus the
 * five helper .apps into Contents/Frameworks. Returns the plan so the signing
 * pass below can walk the same targets. */
async function stageCefMac(
  config: NativeDesktopConfig,
  identity: ResolvedIdentity,
  contents: string,
  hostBinary: string,
  minimumSystemVersion: string,
): Promise<CefMacPlan> {
  if (!identity.id) throw new Error('nd: webview engine "chromium" needs app.id (the CEF helper bundle ids derive from it)');
  const version = cefVersionFor(config.webview?.cef);
  const distRoot = await ensureCefDist({ cefPlatform: cefPlatformKey("darwin", process.arch), version });
  const plan = planCefMac({
    distRoot,
    contents,
    appName: identity.name,
    appId: identity.id,
    version: identity.version,
    minimumSystemVersion,
    helperBinary: await ensureCefHelperBinary(hostBinary),
  });
  applyCefMacPlan(plan);
  console.error(`ND_PACKAGE_CEF ${plan.framework.to} version=${version}`);
  return plan;
}

/** Notarization decision: an explicit flag/config wins; the auto default needs credentials AND a signed bundle. */
export function resolveNotarize(explicit: boolean | undefined, creds: boolean, signed: boolean): boolean {
  return explicit ?? (creds && signed);
}

export async function packageMacApp(
  appDir: string,
  config: NativeDesktopConfig,
  identity: ResolvedIdentity,
  outDir: string,
  options: PackageOptions,
): Promise<PackageResult> {
  const mac = config.package?.mac;
  const dist = join(outDir, "mac");
  const app = join(dist, `${identity.name}.app`);
  const c = join(app, "Contents");
  mkdirSync(join(c, "MacOS"), { recursive: true });
  mkdirSync(join(c, "Resources"), { recursive: true });
  mkdirSync(join(c, "Frameworks"), { recursive: true });

  const engine = resolveWebViewEngine(config, "mac");
  await assemblePayload({
    appDir,
    config,
    identity,
    appRoot: join(c, "Resources", "app"),
    engine,
    schemes: resolveCefSchemes(config),
    entry: options.entry,
    compile: options.compile,
  });

  const hostBinary = await resolveHostBinary({ backend: mac?.backend ?? "appkit" });
  const exe = join(c, "MacOS", identity.name);
  cpSync(hostBinary, exe, { dereference: true });
  chmodSync(exe, 0o755);
  const bunPath = config.package?.bunPath ?? Bun.which("bun");
  if (!bunPath) throw new Error("nd: bun not found on PATH");
  // dereference: true - `bun` on PATH is frequently a symlink into a read-only
  // nix store; copying the link itself would leave a dangling/unwritable entry
  // once relocated into the .app (chmod EPERM on the nix store target).
  cpSync(bunPath, join(c, "MacOS", "bun"), { dereference: true });
  chmodSync(join(c, "MacOS", "bun"), 0o755);

  const minimumSystemVersion = mac?.minimumSystemVersion ?? "26.0";
  const cefPlan = engine === "chromium"
    ? await stageCefMac(config, identity, c, hostBinary, minimumSystemVersion)
    : undefined;

  const iconFile = await installMacIcon(identity, appDir, join(c, "Resources"), minimumSystemVersion);
  const customXml = mac?.infoPlist ? readFileSync(resolve(appDir, mac.infoPlist), "utf8") : undefined;
  writeFileSync(join(c, "Info.plist"), buildInfoPlist(identity, mac, customXml, iconFile));

  // Signing identity: --sign wins, then config, then env, then ad-hoc.
  // --no-sign (signIdentity === null) skips codesign entirely.
  const sign = options.signIdentity !== undefined
    ? options.signIdentity
    : mac?.signIdentity ?? process.env.APPLE_SIGN_IDENTITY ?? "-";
  if (sign !== null) {
    const ent = mac?.entitlements ? resolve(appDir, mac.entitlements) : join(dist, "entitlements.plist");
    if (!mac?.entitlements) writeFileSync(ent, ENTITLEMENTS_PLIST);
    // Deep-sign inside-out: nested Mach-O first, the .app last. `bun` is the
    // process that actually needs allow-jit.
    const nativeDir = join(c, "Resources", "app", "native");
    const plugins = existsSync(nativeDir)
      ? [...new Bun.Glob("*.dylib").scanSync({ cwd: nativeDir, absolute: true })].sort()
      : [];
    if (cefPlan) {
      // The renderer and GPU helpers carry entitlements the app's own do not,
      // and `--deep` re-signs nested code with whatever it was handed, which
      // would drop them. So the CEF payload is enumerated and signed
      // inside-out here, and the outer sign below drops --deep.
      const helperEnt = writeCefHelperEntitlements(dist);
      for (const target of cefMacSignTargets(cefPlan)) {
        if (target.jit) await $`codesign --force --options runtime --entitlements ${helperEnt} --sign ${sign} ${target.path}`;
        else await $`codesign --force --options runtime --sign ${sign} ${target.path}`;
      }
    }
    for (const nested of [join(c, "MacOS", "bun"), exe, ...plugins]) {
      await $`codesign --force --options runtime --entitlements ${ent} --sign ${sign} ${nested}`;
    }
    if (cefPlan) await $`codesign --force --options runtime --entitlements ${ent} --sign ${sign} ${app}`;
    else await $`codesign --force --deep --options runtime --entitlements ${ent} --sign ${sign} ${app}`;
    await $`codesign --verify --strict ${app}`;
    console.error(`ND_PACKAGE_APP_SIGNED ${app} identity=${sign === "-" ? "adhoc" : "developer-id"}`);
  }

  // Notarization: explicit --notarize/--no-notarize/config wins; the default
  // is auto (run only when all three Apple credentials are present AND the
  // bundle was signed - Apple rejects unsigned submissions outright).
  const { APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD } = process.env;
  const creds = !!(APPLE_ID && APPLE_TEAM_ID && APPLE_APP_PASSWORD);
  const notarize = resolveNotarize(options.notarize ?? mac?.notarize, creds, sign !== null);
  if (notarize && sign === null) throw new Error("nd: notarize requested but signing is disabled (--no-sign)");
  if (notarize && !creds) throw new Error("nd: notarize requested but APPLE_ID/APPLE_TEAM_ID/APPLE_APP_PASSWORD are not all set");
  if (notarize) {
    const zip = join(dist, `${identity.name}.zip`);
    await $`ditto -c -k --keepParent ${app} ${zip}`;
    await $`xcrun notarytool submit ${zip} --apple-id ${APPLE_ID} --team-id ${APPLE_TEAM_ID} --password ${APPLE_APP_PASSWORD} --wait`;
    await $`xcrun stapler staple ${app}`;
    console.error(`ND_PACKAGE_NOTARIZE_OK ${app}`);
  } else {
    const reason = options.notarize === false || mac?.notarize === false ? "disabled" : sign === null ? "unsigned" : "no-credentials";
    console.error(`ND_PACKAGE_NOTARIZE_SKIPPED reason=${reason}`);
  }

  const update = await maybePublishUpdate(config.package?.updates, {
    identity,
    platform: "mac",
    distDir: dist,
    member: `${identity.name}.app`,
    outDir,
  });

  console.error(`ND_PACKAGE_OK ${app}`);
  return { bundlePath: app, updateManifest: update?.manifestPath, publicKey: update?.publicKey };
}
