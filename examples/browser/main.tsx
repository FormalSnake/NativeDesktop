import { render, sendCommand, useRef, useState } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

// A very small Min-style browser: a back button and a search/address field in
// the WINDOW's own chrome, and the page — nothing else. The <toolbarview> +
// <headerbar> pair sits directly under <window>, so the chrome row lands in
// the real titlebar on macOS (unified NSToolbar, traffic lights inline) and
// in a real AdwHeaderBar on GTK (window controls included). The page surface
// is the platform's own engine (WKWebView on macOS, WebKitGTK on Linux via
// runtime dlopen; without libwebkitgtk-6.0 installed it degrades to a
// placeholder label). Anything typed that doesn't look like an address
// becomes a DuckDuckGo search, the window title tracks the page title, and
// back drives the engine's real history through sendCommand(). `url` is a
// controlled prop with a host-side echo guard, so feeding onNavigate back
// into state does not re-trigger a load.
const HOME = "https://formalsnake.dev/";

function toUrl(raw: string): string | null {
  const q = raw.trim();
  if (!q) return null;
  if (/^[a-z][a-z0-9+.-]*:/i.test(q)) return q; // already has a scheme
  if (/^\S+\.\S{2,}$/.test(q)) return `https://${q}`; // looks like a host
  return `https://duckduckgo.com/?q=${encodeURIComponent(q)}`;
}

function App(): React.ReactNode {
  const page = useRef<NdNodeRef<"webview">>(null);
  const [url, setUrl] = useState(HOME);
  const [address, setAddress] = useState(HOME);
  const [title, setTitle] = useState("Browser");
  const [canGoBack, setCanGoBack] = useState(false);
  const [canGoForward, setCanGoForward] = useState(false);

  return (
    <window title={title} defaultWidth={960} defaultHeight={640}>
      <toolbarview>
        {/* The header's NATIVE back/forward control (System Settings' `< >`):
            canGoBack/canGoForward mirror the engine's history availability
            (webview onBackAvailable/onForwardAvailable below), and the
            control's onBack/onForward drive the engine via sendCommand. */}
        {/* title="" keeps the toolbar pure chrome (no app-name label) — the
            page title still tracks the WINDOW title for Mission Control. */}
        <headerbar
          title=""
          testID="chrome"
          canGoBack={canGoBack}
          canGoForward={canGoForward}
          onBack={() => { if (page.current) sendCommand(page.current, "goBack"); }}
          onForward={() => { if (page.current) sendCommand(page.current, "goForward"); }}
        >
          <searchinput
            slot="start"
            text={address}
            placeholder="Search or enter address"
            testID="address"
            onChanged={(e) => setAddress(e.text)}
            onActivate={(e) => {
              const target = toUrl(e.text);
              if (target) {
                setAddress(target);
                setUrl(target);
              }
            }}
          />
        </headerbar>
        <webview
          ref={page}
          url={url}
          testID="page"
          style={{ hexpand: true, vexpand: true }}
          onNavigate={(e) => {
            // Track real navigations (link clicks, redirects, history moves)
            // into both the address field and the controlled url prop.
            setAddress(e.text);
            setUrl(e.text);
          }}
          onTitleChanged={(e) => setTitle(e.text || "Browser")}
          onBackAvailable={(e) => setCanGoBack(e.checked)}
          onForwardAvailable={(e) => setCanGoForward(e.checked)}
        />
      </toolbarview>
    </window>
  );
}

await render(<App />);
