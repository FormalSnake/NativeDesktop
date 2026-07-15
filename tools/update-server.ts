#!/usr/bin/env bun
// Local update-manifest server (test fixture). Serves a dir over 127.0.0.1
// only. No network egress; the update flow test never leaves loopback.
import { join, normalize } from "node:path";
const dir = process.argv[2]!, port = Number(process.argv[3] ?? 0);
const server = Bun.serve({
  hostname: "127.0.0.1", port,
  fetch(req) {
    const path = normalize(new URL(req.url).pathname).replace(/^(\.\.[/\\])+/, "");
    const file = Bun.file(join(dir, path));
    return new Response(file);
  },
});
console.log(`ND_UPDATE_SERVER_LISTENING port=${server.port}`);
