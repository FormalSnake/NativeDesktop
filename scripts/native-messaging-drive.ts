// Asks the native messaging fixture extension's worker to message
// dev.nativedesktop.echo and checks the host's reply. The gates put that
// host's manifest where Chrome looks, never where Chromium does.
// Marker: ND_NATIVE_MESSAGING_OK.
const port = process.env.ND_CDP_PORT ?? process.env.ND_CEF_DEBUG_PORT ?? "9334";
const WORKER = "chrome-extension://poekbdjmocmcnaipbndfoomkigdelckf/background.js";

type Target = { type: string; url: string; webSocketDebuggerUrl?: string };

async function workerTarget(): Promise<Target> {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    try {
      const list = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()) as Target[];
      const hit = list.find((t) => t.type === "service_worker" && t.url === WORKER && t.webSocketDebuggerUrl);
      if (hit) return hit;
    } catch {}
    await Bun.sleep(250);
  }
  throw new Error(`no service worker target ${WORKER} on port ${port}`);
}

function evaluate(wsUrl: string, expression: string): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(wsUrl);
    const timer = setTimeout(() => {
      ws.close();
      reject(new Error("Runtime.evaluate timed out"));
    }, 20_000);
    ws.onopen = () =>
      ws.send(JSON.stringify({ id: 1, method: "Runtime.evaluate", params: { expression, awaitPromise: true, returnByValue: true } }));
    ws.onmessage = (event) => {
      const msg = JSON.parse(String(event.data));
      if (msg.id !== 1) return;
      clearTimeout(timer);
      ws.close();
      if (msg.error || msg.result?.exceptionDetails) reject(new Error(JSON.stringify(msg.error ?? msg.result.exceptionDetails)));
      else resolve(msg.result.result.value);
    };
    ws.onerror = () => reject(new Error(`websocket to ${wsUrl} failed`));
  });
}

const target = await workerTarget();
// The target is listed before background.js has finished running on a slow
// host, so the call waits for the worker to define it.
let reply: unknown = "not-ready";
for (let i = 0; i < 40 && reply === "not-ready"; i++) {
  reply = await evaluate(target.webSocketDebuggerUrl!, "typeof ndPing === 'function' ? ndPing() : 'not-ready'");
  if (reply === "not-ready") await Bun.sleep(250);
}
if (reply === '{"pong":true}') {
  console.log(`ND_NATIVE_MESSAGING_OK reply=${reply}`);
} else {
  console.log(`FAIL: native messaging reply ${JSON.stringify(reply)}`);
  process.exit(1);
}
