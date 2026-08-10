#!/usr/bin/env bun
// Legacy entrypoint: `bun tools/package.ts <mac|linux>` packages the gallery
// example through the real `nd package` implementation
// (packages/nd/src/package/). Kept so headless-m9.sh / mac-m9.sh /
// package.yml invocations keep working unchanged.
import { packageApp } from "../packages/nd/src/package/index.ts";

const platform = process.argv[2];
if (platform !== "linux" && platform !== "mac") {
  console.error("usage: bun tools/package.ts <linux|mac>  (Windows lands with M7)");
  process.exit(2);
}
await packageApp({ platform, cwd: "examples/gallery", outDir: "dist" });
