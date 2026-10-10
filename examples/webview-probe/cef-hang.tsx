import { render } from "@nativedesktop/react";
import { createSignal } from "solid-js";

// "Page Unresponsive" gate, driven by scripts/cef-hang-drive.ts. A page that
// is only waiting (on a navigation to a server that never answers, on a new
// renderer for another site, on its own alert) must never be asked about; a
// page whose main thread spins must be, and Wait and Exit page must work.
const html = (title: string, body = "") =>
  new Response(
    `<!doctype html><html><head><meta charset="utf-8" /><title>${title}</title>
<style>body{font:20px system-ui;margin:0;padding:24px;background:#fff;color:#111}</style>
</head><body><h1>${title}</h1>${body}</body></html>`,
    { headers: { "content-type": "text/html; charset=utf-8" } },
  );

const fixture = Bun.serve({
  port: 0,
  hostname: "127.0.0.1",
  idleTimeout: 0,
  async fetch(request) {
    const path = new URL(request.url).pathname;
    if (path === "/slow") {
      await Bun.sleep(60_000);
      return html("slow answered");
    }
    if (path === "/alert") return html("alert", `<script>setTimeout(() => alert("waiting on you"), 300)</script>`);
    if (path === "/busy") return html("busy", `<script>setTimeout(() => { for (;;) {} }, 1000)</script>`);
    return html(`home ${new URL(request.url).hostname}`);
  },
});
// 127.0.0.1 and localhost are different sites, so moving between them puts
// the page in a new renderer.
const SAME = `http://127.0.0.1:${fixture.port}`;
const OTHER = `http://localhost:${fixture.port}`;

const targets: Record<string, string> = {
  home: `${SAME}/`,
  slow: `${SAME}/slow`,
  crossSlow: `${OTHER}/slow`,
  cross: `${OTHER}/`,
  neverssl: "http://neverssl.com/",
  alert: `${SAME}/alert`,
  busy: `${SAME}/busy`,
};

function App() {
  const [url, setUrl] = createSignal(targets.home!);
  const [title, setTitle] = createSignal("");
  const [gone, setGone] = createSignal("");
  return (
    <window title="ND CEF hang" defaultWidth={900} defaultHeight={640}>
      <box orientation="vertical" spacing={4} style={{ padding: 12 }}>
        <label testID="h-title" text={`title=${title()}`} />
        <label testID="h-gone" text={`gone=${gone()}`} />
        <box orientation="horizontal" spacing={8}>
          {Object.keys(targets).map((name) => (
            <button testID={`h-${name}`} label={name} onClick={() => setUrl(targets[name]!)} />
          ))}
        </box>
        <webview
          testID="h-view"
          url={url()}
          style={{ vexpand: true, hexpand: true }}
          onTitleChanged={(e) => setTitle(String(e.text ?? ""))}
          onRenderProcessGone={(e) => setGone(JSON.stringify(e.data))}
        />
      </box>
    </window>
  );
}

await render(() => <App />);
