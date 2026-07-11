#!/usr/bin/env bun
// nd package <platform>  (nd convention: nd package ≡ bun tools/package.ts).
// Linux → AppImage + signed update archive/manifest (M9-D4).
// mac   → .app + deep codesign (ad-hoc or Developer-ID) + gated notarize (M9-D3).
import { packageLinux } from "./package-linux.ts";

const platform = process.argv[2];
if (platform === "linux") {
  await packageLinux();
} else if (platform === "mac") {
  // Dynamic import so the linux path never loads tools/package-mac.ts
  // (owned by a parallel task; may not exist yet at package.ts's own
  // load time on non-mac dev/CI legs).
  const { packageMac } = await import("./package-mac.ts");
  await packageMac();
} else {
  console.error("usage: bun tools/package.ts <linux|mac>  (Windows lands with M7)");
  process.exit(2);
}
