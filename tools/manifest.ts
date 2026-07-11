#!/usr/bin/env bun
// tools/manifest.ts — shared minisign-signing manifest builder (M9).
// Shells out to the `minisign` CLI (on PATH from the devshell). Produces the
// update manifest JSON matching src/core/update.zig's `Manifest` struct and
// signs both the archive and the manifest.
import { $ } from "bun";
import { readFileSync, writeFileSync, existsSync } from "node:fs";

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
  // The second line of the .minisig is the base64 algo‖keyid‖sig blob the Zig
  // verifier decodes directly — pass it through as full_sig_b64.
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
