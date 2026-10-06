import { render } from "@nativedesktop/solid";
import { For, Show, createSignal } from "solid-js";

// A terminal app with REAL native chrome AND native system tabs — the
// Ghostty setup: every tab is its own <window tabGroup="terminal"> root, so
// macOS shows Safari/Finder-style window tabs and GNOME gets an AdwTabBar
// under the header plus the AdwTabOverview button. Each tab runs its own
// independent shell; the native "+" fires onNewTabRequested and a user close
// fires onClosed, so the app only manages a list of tab ids. Dragging a tab
// out into its own window (or back in) is native on both platforms and never
// touches the Solid tree — the running shell just moves.
//
// The <terminal> widget hosts a native drawing surface (GtkDrawingArea on GTK,
// NDTerminalView/CoreText on AppKit) driven by libghostty-vt over the ndterm core:
// a PTY runs $SHELL and its output is parsed into the cell grid, with keystrokes
// fed straight back to the PTY host-side.
function TerminalTab(props: { id: number; withMenu: boolean; onNewTab: () => void; onClose: () => void }) {
  return (
    <window
      title={props.id === 0 ? "Terminal" : `Terminal — ${props.id + 1}`}
      defaultWidth={860}
      defaultHeight={560}
      tabGroup="terminal"
      onNewTabRequested={() => props.onNewTab()}
      onClosed={() => props.onClose()}
    >
      {/* Process-wide app menu on the first open tab (same move as the
          browser example); File > Close from `defaults` closes a tab. */}
      <Show when={props.withMenu}>
        <menubar defaults>
          <menu label="File" testID="menu-file">
            <menuitem testID="menu-new-tab" label="New Tab" accelerator="primary+t" onSelect={() => props.onNewTab()} />
          </menu>
        </menubar>
      </Show>
      <toolbarview>
        <headerbar title="Terminal" testID="chrome" />
        {/* <terminal> is the pane's own content, sized by its own hexpand/
            vexpand: no wrapper box needed to fill the pane. */}
        <terminal
          cols={100}
          rows={30}
          fontSize={13}
          style={{ hexpand: true, vexpand: true }}
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
        <TerminalTab
          id={id}
          withMenu={i() === 0}
          onNewTab={addTab}
          onClose={() => setTabs((open) => open.filter((t) => t !== id))}
        />
      )}
    </For>
  );
}

await render(() => <App />);
