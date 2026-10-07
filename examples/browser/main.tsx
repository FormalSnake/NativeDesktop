import { render, sendCommand } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";
import { For, Show, createSignal } from "solid-js";

// A very small Min-style browser with NATIVE system tabs: every tab is its
// own <window tabGroup="browser"> root, so macOS groups them as real
// NSWindow tabs (Safari-style — drag a tab out to its own window, drag it
// back in, Show All Tabs) and GNOME renders an AdwTabBar under the header
// with an AdwTabOverview button in it. All tab chrome comes from the
// framework; the app only owns the LIST of tabs. Chrome-style drag and drop
// between windows is entirely native — the OS moves the window/page, the
// Solid tree (and each tab's live webview, history and all) never changes.
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

function BrowserTab(props: { withMenu: boolean; onNewTab: () => void; onClose: () => void }) {
  let page: NdNodeRef<"webview"> | undefined;
  const [url, setUrl] = createSignal(HOME);
  const [address, setAddress] = createSignal(HOME);
  const [title, setTitle] = createSignal("New Tab");
  const [canGoBack, setCanGoBack] = createSignal(false);
  const [canGoForward, setCanGoForward] = createSignal(false);

  return (
    <window
      title={title()}
      defaultWidth={960}
      defaultHeight={640}
      tabGroup="browser"
      onNewTabRequested={() => props.onNewTab()}
      onClosed={() => props.onClose()}
    >
      {/* App menu is process-wide chrome; exactly one window may own it, so
          it rides the FIRST open tab and re-attaches if that tab closes. Ctrl+W
          is a native tab-system binding (closes the active tab from any tab);
          the menu entry stays mouse-only because a menu accelerator registers
          app-globally and would always close this menu-owning tab instead. */}
      <Show when={props.withMenu}>
        <menubar defaults>
          <menu label="File" testID="menu-file">
            <menuitem testID="menu-new-tab" label="New Tab" accelerator="primary+t" onSelect={() => props.onNewTab()} />
            <menuitem testID="menu-close-tab" label="Close Tab" onSelect={() => props.onClose()} />
          </menu>
        </menubar>
      </Show>
      <toolbarview>
        {/* title="" keeps the toolbar pure chrome (no app-name label) — the
            page title still tracks the WINDOW title, which native tabbing
            reuses as the TAB title on both platforms. */}
        <headerbar
          title=""
          testID="chrome"
          canGoBack={canGoBack()}
          canGoForward={canGoForward()}
          onBack={() => { if (page) sendCommand(page, "goBack"); }}
          onForward={() => { if (page) sendCommand(page, "goForward"); }}
        >
          <searchinput
            slot="start"
            text={address()}
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
          url={url()}
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

function App() {
  const [tabs, setTabs] = createSignal<number[]>([0]);
  let nextId = 1;
  const addTab = () => setTabs((open) => [...open, nextId++]);

  return (
    <For each={tabs()}>
      {(id, i) => (
        <BrowserTab
          withMenu={i() === 0}
          onNewTab={addTab}
          onClose={() => setTabs((open) => open.filter((t) => t !== id))}
        />
      )}
    </For>
  );
}

await render(() => <App />);
