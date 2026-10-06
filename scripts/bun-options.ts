#!/usr/bin/env bun
// scripts/bun-options.ts <entry>: prints the BUN_OPTIONS a raw host launch of
// <entry> needs (the renderer preload for a Solid app), the value nd dev and
// launchApp set. Shell gates that start a host by hand export it:
//   export BUN_OPTIONS="$(bun scripts/bun-options.ts examples/x/main.tsx)"
import { dirname, resolve } from "node:path";
import { findAppDir, preloadEnv } from "@nativedesktop/host";

const entry = process.argv[2];
if (!entry) throw new Error("usage: bun scripts/bun-options.ts <entry>");
const appDir = findAppDir(dirname(resolve(entry)));
console.log((appDir ? preloadEnv(appDir).BUN_OPTIONS : undefined) ?? process.env.BUN_OPTIONS ?? "");
