import { render, sendCommand, useRef, useState } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

// A minimal web browser built on the <webview> widget (M14): the page surface
// is the PLATFORM's own engine — WKWebView on macOS, WebKitGTK on Linux (a
// runtime dlopen; without libwebkitgtk-6.0 installed the widget degrades to a
// placeholder label). Chrome is ordinary framework widgets: an address bar
// (<textinput>), back/forward/reload <button>s driving the widget through
// sendCommand(), and a <spinner> fed by onLoadingChanged. The window title
// tracks the page title. `url` is a controlled prop with a host-side echo
// guard, so feeding onNavigate back into state does not re-trigger a load.
const HOME = "https://example.com/";

function App(): React.ReactNode {
  const webview = useRef<NdNodeRef<"webview">>(null);
  const [url, setUrl] = useState(HOME);
  const [address, setAddress] = useState(HOME);
  const [title, setTitle] = useState("NativeDesktop Browser");
  const [loading, setLoading] = useState(false);

  const navigate = (raw: string) => {
    const trimmed = raw.trim();
    if (!trimmed) return;
    const target = /^[a-z][a-z0-9+.-]*:/i.test(trimmed) ? trimmed : `https://${trimmed}`;
    setAddress(target);
    setUrl(target);
  };

  return (
    <window title={title} defaultWidth={1000} defaultHeight={700}>
      <box orientation="vertical" spacing={0}>
        <box orientation="horizontal" spacing={6} style={{ padding: 8 }}>
          <button
            label="←"
            testID="nav-back"
            onClick={() => { if (webview.current) sendCommand(webview.current, "goBack"); }}
          />
          <button
            label="→"
            testID="nav-forward"
            onClick={() => { if (webview.current) sendCommand(webview.current, "goForward"); }}
          />
          <button
            label={loading ? "✕" : "⟳"}
            testID="nav-reload"
            onClick={() => { if (webview.current) sendCommand(webview.current, loading ? "stop" : "reload"); }}
          />
          <textinput
            text={address}
            placeholder="Enter a URL"
            testID="address-bar"
            style={{ hexpand: true }}
            onChanged={(e) => setAddress(e.text)}
            onActivate={(e) => navigate(e.text)}
          />
          <spinner spinning={loading} />
        </box>
        <separator />
        <webview
          ref={webview}
          url={url}
          testID="page"
          style={{ hexpand: true, vexpand: true }}
          onNavigate={(e) => {
            // Track real navigations (link clicks, redirects, history moves)
            // into both the address bar and the controlled url prop.
            setAddress(e.text);
            setUrl(e.text);
          }}
          onTitleChanged={(e) => setTitle(e.text || "NativeDesktop Browser")}
          onLoadingChanged={(e) => setLoading(e.checked)}
        />
      </box>
    </window>
  );
}

await render(<App />);
