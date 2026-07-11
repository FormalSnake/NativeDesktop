#!/usr/bin/env bun
// Update-flow driver (M9-D5): fetch a signed manifest+archive from the local
// server, run the NON-DISABLEABLE Zig verifier (zig-out/bin/nd-update-verify),
// assert valid=accept + tampered=reject, then atomic-swap-stage. Zero network.
import { $ } from "bun";
import { writeFileSync, mkdirSync, renameSync } from "node:fs";

const [serverUrl, pubKey, stageDir] = process.argv.slice(2);
const version = process.env.ND_APP_VERSION ?? "0.9.0";
const tmp = `${stageDir}/.staging`; mkdirSync(tmp, { recursive: true });

// 1. Fetch the manifest + archive over loopback.
const manifestJson = await (await fetch(`${serverUrl}/manifest-${version}.json`)).text();
const manifest = JSON.parse(manifestJson);
const archiveName = manifest.full_url.split("/").pop()!;
const archiveBytes = new Uint8Array(await (await fetch(`${serverUrl}/${archiveName}`)).arrayBuffer());
const archivePath = `${tmp}/${archiveName}`;
writeFileSync(archivePath, archiveBytes);

// 2. Reconstruct a minisign .minisig file from manifest.full_sig_b64 so the Zig
//    CLI (which reads a .minisig second line) can verify. Line 1 = comment.
const sigPath = `${archivePath}.minisig`;
writeFileSync(sigPath, `untrusted comment: nd update\n${manifest.full_sig_b64}\n`);

// 3. Run the NON-DISABLEABLE verifier on the VALID archive → must exit 0.
const verify = "zig-out/bin/nd-update-verify";
const okProc = await $`${verify} --pubkey ${pubKey} --message ${archivePath} --sig ${sigPath}`.nothrow();
if (okProc.exitCode !== 0) { console.error("M9_UPDATE_FAIL valid archive rejected"); process.exit(1); }

// 4. Tamper the archive → verifier MUST reject (exit 1).
const tamperedPath = `${tmp}/tampered-${archiveName}`;
const tampered = new Uint8Array(archiveBytes); tampered[tampered.length - 1] ^= 0xFF;
writeFileSync(tamperedPath, tampered);
const badProc = await $`${verify} --pubkey ${pubKey} --message ${tamperedPath} --sig ${sigPath}`.nothrow();
if (badProc.exitCode === 0) { console.error("M9_UPDATE_FAIL tampered archive ACCEPTED — security hole"); process.exit(1); }

// 5. Atomic-swap staging: only a verified archive is promoted.
const staged = `${stageDir}/${archiveName}`;
renameSync(archivePath, staged);
console.log(`M9_UPDATE_OK verified+staged version=${version} staged=${staged}`);
