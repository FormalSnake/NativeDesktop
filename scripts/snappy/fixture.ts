// Flat-colour page (first paint is easy to see from outside), plus numbered
// pages with their own favicon, so a reload also redraws the tab row's icon.
const port = Number(process.argv[2]);
const icon = Uint8Array.from(
  atob("iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAIAAACQkWg2AAAAFklEQVR42mN4FqVBEmIY1TCqYfhqAAAhaGgQjjUPFwAAAABJRU5ErkJggg=="),
  (c) => c.charCodeAt(0),
);
Bun.serve({
  port,
  hostname: "127.0.0.1",
  fetch(req) {
    const u = new URL(req.url);
    if (u.pathname === "/icon.png") return new Response(icon, { headers: { "content-type": "image/png", "cache-control": "no-store" } });
    const n = u.searchParams.get("tab") ?? "";
    return new Response(
      `<!doctype html><title>Fixture ${n}</title><link rel="icon" href="/icon.png?tab=${n}"><body style="margin:0;background:#2a6fdb;font:24px sans-serif;color:#fff"><p style="padding:40px">Fixture ${n}</p></body>`,
      { headers: { "content-type": "text/html; charset=utf-8" } },
    );
  },
});
