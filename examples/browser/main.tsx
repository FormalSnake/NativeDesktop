import { render, sendCommand, useRef, useState } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

// A very small Min-style browser with NATIVE system tabs: every tab is its
// own <window tabGroup="browser"> root, so macOS groups them as real
// NSWindow tabs (Safari-style — drag a tab out to its own window, drag it
// back in, Show All Tabs) and GNOME renders an AdwTabBar under the header
// with an AdwTabOverview button in it. All tab chrome comes from the
// framework; the app only owns the LIST of tabs. Chrome-style drag and drop
// between windows is entirely native — the OS moves the window/page, the
// React tree (and each tab's live webview, history and all) never changes.
//
// The native "+" (tab bar / Cmd+T target) fires onNewTabRequested -> append
// an id; a user close fires onClosed -> drop the id, which unmounts that
// <window> and confirms the native close. Each tab is otherwise the same
// mini browser as before: headerbar back/forward + address field in the real
// titlebar, page title tracks the window (= tab) title.
const HOME = "https://formalsnake.dev/";

function toUrl(raw: string): string | null {
  const q = raw.trim();
  if (!q) return null;
  if (/^[a-z][a-z0-9+.-]*:/i.test(q)) return q; // already has a scheme
  if (/^\S+\.\S{2,}$/.test(q)) return `https://${q}`; // looks like a host
  return `https://duckduckgo.com/?q=${encodeURIComponent(q)}`;
}

function BrowserTab({ withMenu, onNewTab, onClose }: { withMenu: boolean; onNewTab: () => void; onClose: () => void }): React.ReactNode {
  const page = useRef<NdNodeRef<"webview">>(null);
  const [url, setUrl] = useState(HOME);
  const [address, setAddress] = useState(HOME);
  const [title, setTitle] = useState("New Tab");
  const [canGoBack, setCanGoBack] = useState(false);
  const [canGoForward, setCanGoForward] = useState(false);

  return (
    <window
      title={title}
      defaultWidth={960}
      defaultHeight={640}
      tabGroup="browser"
      onNewTabRequested={onNewTab}
      onClosed={onClose}
    >
      {/* App menu is process-wide chrome; exactly one window may own it, so
          it rides the FIRST open tab and re-attaches if that tab closes. Ctrl+W
          is a native tab-system binding (closes the active tab from any tab);
          the menu entry stays mouse-only because a menu accelerator registers
          app-globally and would always close this menu-owning tab instead. */}
      {withMenu && (
        <menubar defaults>
          <menu label="File" testID="menu-file">
            <menuitem testID="menu-new-tab" label="New Tab" accelerator="primary+t" onSelect={onNewTab} />
            <menuitem testID="menu-close-tab" label="Close Tab" onSelect={onClose} />
          </menu>
        </menubar>
      )}
      <toolbarview>
        {/* title="" keeps the toolbar pure chrome (no app-name label) — the
            page title still tracks the WINDOW title, which native tabbing
            reuses as the TAB title on both platforms. */}
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
          onTitleChanged={(e) => setTitle(e.text || "New Tab")}
          onBackAvailable={(e) => setCanGoBack(e.checked)}
          onForwardAvailable={(e) => setCanGoForward(e.checked)}
        />
      </toolbarview>
    </window>
  );
}

function App(): React.ReactNode {
  const [tabs, setTabs] = useState<number[]>([0]);
  const nextId = useRef(1);
  const addTab = () => setTabs((open) => [...open, nextId.current++]);

  return (
    <>
      {tabs.map((id, i) => (
        <BrowserTab
          key={id}
          withMenu={i === 0}
          onNewTab={addTab}
          onClose={() => setTabs((open) => open.filter((t) => t !== id))}
        />
      ))}
    </>
  );
}

await render(<App />);
