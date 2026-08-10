// Update artifacts: minisign-signed full archive + manifest JSON matching
// src/core/update.zig's `Manifest` struct. Shells out to the `minisign` CLI.
// Opt-in per app: no `package.updates` config, no artifacts.
import { $ } from "bun";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";
import type { UpdatesConfig } from "../config.ts";
import type { ResolvedIdentity } from "./identity.ts";

export interface ManifestOpts {
  appId: string;
  version: string;
  from: string | null;
  archivePath: string;
  url: string;
  secKey: string;
  pubKey: string;
}

// Returns { manifestPath, manifestSigPath }.
export async function buildAndSignManifest(o: ManifestOpts) {
  // Sign the archive (minisign -S writes <archive>.minisig).
  await $`minisign -S -s ${o.secKey} -m ${o.archivePath}`.quiet();
  const archiveSig = `${o.archivePath}.minisig`;
  // The second line of the .minisig is the base64 algo/keyid/sig blob the Zig
  // verifier decodes directly; pass it through as full_sig_b64.
  const sigB64 = readFileSync(archiveSig, "utf8").split("\n")[1]!.trim();
  const manifest = {
    app_id: o.appId,
    version: o.version,
    from: o.from,
    full_url: o.url,
    full_sig_b64: sigB64,
  };
  const manifestPath = `${o.archivePath.replace(/[^/]+$/, "")}manifest-${o.version}.json`;
  writeFileSync(manifestPath, JSON.stringify(manifest, null, 2));
  await $`minisign -S -s ${o.secKey} -m ${manifestPath}`.quiet();
  return { manifestPath, manifestSigPath: `${manifestPath}.minisig` };
}

// Ephemeral key helper for CI/tests (no real signing identity in this env).
// Reads ND_MINISIGN_SEC/ND_MINISIGN_PUB if set; otherwise generates a
// throwaway no-password keypair under `dir`.
export async function ensureEphemeralKey(dir: string) {
  if (process.env.ND_MINISIGN_SEC && process.env.ND_MINISIGN_PUB) {
    return { sec: process.env.ND_MINISIGN_SEC, pub: process.env.ND_MINISIGN_PUB };
  }
  const sec = `${dir}/nd-sign.sec`, pub = `${dir}/nd-sign.pub`;
  if (!existsSync(sec)) await $`minisign -G -W -p ${pub} -s ${sec}`.quiet();
  return { sec, pub };
}

export interface PublishOptions {
  identity: ResolvedIdentity;
  platform: "mac" | "linux";
  /** dist/<platform> directory the archive member lives in. */
  distDir: string;
  /** The archive member inside distDir (Gallery.app / AppDir). */
  member: string;
  /** Resolved output root; update artifacts land in <outDir>/update. */
  outDir: string;
}

export interface PublishResult {
  manifestPath: string;
  publicKey: string;
}

/**
 * Archives + signs when `package.updates` is configured; otherwise prints
 * ND_PACKAGE_UPDATES_SKIPPED and returns undefined. Tar invocations stay
 * exactly as the update flow's consumers expect (.tar.gz mac / .tar.zst linux,
 * zstd via the CLI).
 */
export async function maybePublishUpdate(cfg: UpdatesConfig | undefined, o: PublishOptions): Promise<PublishResult | undefined> {
  if (!cfg) {
    console.error("ND_PACKAGE_UPDATES_SKIPPED reason=not-configured");
    return undefined;
  }
  if (!o.identity.id) throw new Error("nd: package.updates needs app.id (the manifest's app_id)");

  const updDir = join(o.outDir, "update");
  mkdirSync(updDir, { recursive: true });
  const format = cfg.format ?? (o.platform === "mac" ? "tar.gz" : "tar.zst");
  const archive = join(updDir, `${o.identity.slug}-${o.identity.version}-${o.platform}.${format}`);
  rmSync(archive, { force: true });
  if (format === "tar.gz") {
    await $`tar -C ${o.distDir} -czf ${archive} ${o.member}`;
  } else {
    await $`tar -C ${o.distDir} -cf - ${o.member} | zstd -q -o ${archive}`.quiet();
  }

  let sec: string, pub: string;
  if (cfg.secretKey) {
    if (!cfg.publicKey) throw new Error("nd: package.updates.secretKey needs publicKey too");
    sec = cfg.secretKey;
    pub = cfg.publicKey;
  } else if (process.env.ND_MINISIGN_SEC && process.env.ND_MINISIGN_PUB) {
    sec = process.env.ND_MINISIGN_SEC;
    pub = process.env.ND_MINISIGN_PUB;
  } else if (cfg.ephemeralKey) {
    ({ sec, pub } = await ensureEphemeralKey(updDir));
  } else {
    throw new Error("nd: package.updates has no signing key (set secretKey/publicKey, ND_MINISIGN_SEC/ND_MINISIGN_PUB, or ephemeralKey: true)");
  }

  const { manifestPath } = await buildAndSignManifest({
    appId: o.identity.id,
    version: o.identity.version,
    from: null,
    archivePath: archive,
    url: `${cfg.baseUrl.replace(/\/$/, "")}/${basename(archive)}`,
    secKey: sec,
    pubKey: pub,
  });
  console.error(`ND_PACKAGE_MANIFEST ${manifestPath} pub=${pub}`);
  return { manifestPath, publicKey: pub };
}
