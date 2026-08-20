import { render, sendCommand, useEffect, useRef, useState, webviewEngine } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

// Bench for the reserved-scheme question: can a scheme handler factory serve
// chrome-extension://, which Chromium claims for its own extension loader?
// A control scheme runs beside it so a failure can be told apart from the
// scheme machinery being broken outright.
const PAGE = `<!doctype html><html><head><meta charset="utf-8" /><title>ND SCHEME</title></head>
<body><div id="marker">scheme-ok</div></body></html>`;

const EXT_ID = "aaaabbbbccccddddeeeeffffgggghhhh";

function App() {
  const control = useRef<NdNodeRef<"webview"> | null>(null);
  const extension = useRef<NdNodeRef<"webview"> | null>(null);
  const [ready, setReady] = useState(false);
  const [registration, setRegistration] = useState("pending");
  const [requests, setRequests] = useState<string[]>([]);
  const [controlState, setControlState] = useState("pending");
  const [extensionState, setExtensionState] = useState("pending");

  useEffect(() => {
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
  }, []);

  const answer = (ref: NdNodeRef<"webview"> | null, data: unknown): void => {
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
        <label testID="s-register" text={`register=${registration}`} />
        <label testID="s-requests" text={`requests=${requests.join(",") || "none"}`} />
        <label testID="s-control" text={`control=${controlState}`} />
        <label testID="s-extension" text={`extension=${extensionState}`} />
        {ready ? (
          <box orientation="horizontal" spacing={8} style={{ vexpand: true }}>
            <webview
              ref={control}
              testID="s-wv-control"
              url="ndtest://probe/index.html"
              onSchemeRequest={(e) => answer(control.current, e.data)}
              onNavigate={(e) => setControlState(`nav ${e.text}`)}
              onLoadFailed={(e) => setControlState(`failed ${JSON.stringify(e.data)}`)}
            />
            <webview
              ref={extension}
              testID="s-wv-extension"
              url={`chrome-extension://${EXT_ID}/index.html`}
              onSchemeRequest={(e) => answer(extension.current, e.data)}
              onNavigate={(e) => setExtensionState(`nav ${e.text}`)}
              onLoadFailed={(e) => setExtensionState(`failed ${JSON.stringify(e.data)}`)}
            />
          </box>
        ) : null}
      </box>
    </window>
  );
}

await render(<App />);
