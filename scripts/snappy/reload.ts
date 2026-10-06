// Load storm: every page target whose URL starts with <prefix> is reloaded
// every <interval> ms for <duration> ms, over the browser's DevTools port.
//   bun reload.ts <cdp-port> <prefix> <duration-ms> <interval-ms>
const [port, prefix, durationMs, intervalMs] = process.argv.slice(2) as [string, string, string, string];
const targets = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()) as { type: string; url: string; webSocketDebuggerUrl?: string }[];
const pages = targets.filter((t) => t.type === "page" && t.url.startsWith(prefix) && t.webSocketDebuggerUrl);
const sockets = await Promise.all(
  pages.map(
    (p) =>
      new Promise<WebSocket>((resolve, reject) => {
        const ws = new WebSocket(p.webSocketDebuggerUrl!);
        ws.onopen = () => resolve(ws);
        ws.onerror = reject;
      }),
  ),
);
console.error(`ND_RELOAD pages=${sockets.length}`);
let id = 1;
let sent = 0;
const end = Date.now() + Number(durationMs);
while (Date.now() < end) {
  for (const ws of sockets) ws.send(JSON.stringify({ id: id++, method: "Page.reload", params: { ignoreCache: true } }));
  sent += sockets.length;
  await Bun.sleep(Number(intervalMs));
}
for (const ws of sockets) ws.close();
console.error(`ND_RELOAD sent=${sent}`);
