#!/usr/bin/env bun
// The page scripts/headless-app-chrome.sh points the app at. Local rather than
// a real site so the gate has no network dependency and the drive can assert
// exact element rects.
import { join } from "node:path";

const port = Number(process.argv[2] ?? "9557");
const page = Bun.file(join(import.meta.dir, "fixtures", "app-chrome", "index.html"));
// A real favicon, because a tab button that has one is built differently from
// one that does not, and the narrow-window leg needs both.
const icon = Bun.file(join(import.meta.dir, "fixtures", "chrome-ext", "icon16.png"));

Bun.serve({
  port,
  hostname: "127.0.0.1",
  fetch: (req) =>
    new URL(req.url).pathname === "/favicon.ico"
      ? new Response(icon, { headers: { "content-type": "image/png" } })
      : new Response(page, { headers: { "content-type": "text/html; charset=utf-8" } }),
});
console.log(`fixture on http://127.0.0.1:${port}/`);
