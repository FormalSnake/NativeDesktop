import { render, sendCommand, webviewEngine } from "@nativedesktop/solid";
import type { NdNodeRef } from "@nativedesktop/solid";
import { Show, createSignal, onSettled } from "solid-js";

// Bench for the reserved-scheme question: can a scheme handler factory serve
// chrome-extension://, which Chromium claims for its own extension loader?
// A control scheme runs beside it so a failure can be told apart from the
// scheme machinery being broken outright.
const PAGE = `<!doctype html><html><head><meta charset="utf-8" /><title>ND SCHEME</title></head>
<body><div id="marker">scheme-ok</div></body></html>`;

const EXT_ID = "aaaabbbbccccddddeeeeffffgggghhhh";

function App() {
  const [control, setControl] = createSignal<NdNodeRef<"webview">>();
  const [extension, setExtension] = createSignal<NdNodeRef<"webview">>();
  const [ready, setReady] = createSignal(false);
  const [registration, setRegistration] = createSignal("pending");
  const [requests, setRequests] = createSignal<string[]>([]);
  const [controlState, setControlState] = createSignal("pending");
  const [extensionState, setExtensionState] = createSignal("pending");

  onSettled(() => {
    Promise.allSettled([
      webviewEngine.registerScheme("ndtest"),
      webviewEngine.registerScheme("chrome-extension"),
    ]).then((results) => {
      setRegistration(
        results
          .map((r, i) => `${i === 0 ? "ndtest" : "chrome-extension"}=${r.status === "fulfilled" ? "ok" : `fail:${(r.reason as Error).message}`}`)
          .join(" "),
      );
      setReady(true);
    });
  });

  const answer = (ref: NdNodeRef<"webview"> | undefined, data: unknown): void => {
    const request = data as { id: string; url: string; scheme: string };
    setRequests((prev) => [...prev, request.url]);
    if (!ref) return;
    sendCommand(ref, "respondScheme", {
      id: request.id,
      base64: Buffer.from(PAGE).toString("base64"),
      mime: "text/html",
      status: 200,
    });
  };

  return (
    <window title="ND CEF Scheme" defaultWidth={900} defaultHeight={520}>
      <box orientation="vertical" spacing={4} style={{ padding: 12 }}>
        <label testID="s-register" text={`register=${registration()}`} />
        <label testID="s-requests" text={`requests=${requests().join(",") || "none"}`} />
        <label testID="s-control" text={`control=${controlState()}`} />
        <label testID="s-extension" text={`extension=${extensionState()}`} />
        <Show when={ready()}>
          <box orientation="horizontal" spacing={8} style={{ vexpand: true }}>
            <webview
              ref={setControl}
              testID="s-wv-control"
              url="ndtest://probe/index.html"
              onSchemeRequest={(e) => answer(control(), e.data)}
              onNavigate={(e) => setControlState(`nav ${e.text}`)}
              onLoadFailed={(e) => setControlState(`failed ${JSON.stringify(e.data)}`)}
            />
            <webview
              ref={setExtension}
              testID="s-wv-extension"
              url={`chrome-extension://${EXT_ID}/index.html`}
              onSchemeRequest={(e) => answer(extension(), e.data)}
              onNavigate={(e) => setExtensionState(`nav ${e.text}`)}
              onLoadFailed={(e) => setExtensionState(`failed ${JSON.stringify(e.data)}`)}
            />
          </box>
        </Show>
      </box>
    </window>
  );
}

await render(() => <App />);
