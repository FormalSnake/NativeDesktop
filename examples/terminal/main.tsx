import { render, useRef, useState } from "@nativedesktop/react";

// A terminal app with REAL native chrome AND native system tabs — the
// Ghostty setup: every tab is its own <window tabGroup="terminal"> root, so
// macOS shows Safari/Finder-style window tabs and GNOME gets an AdwTabBar
// under the header plus the AdwTabOverview button. Each tab runs its own
// independent shell; the native "+" fires onNewTabRequested and a user close
// fires onClosed, so the app only manages a list of tab ids. Dragging a tab
// out into its own window (or back in) is native on both platforms and never
// touches the React tree — the running shell just moves.
//
// The <terminal> widget hosts a native drawing surface (GtkDrawingArea on GTK,
// NDTerminalView/CoreText on AppKit) driven by libghostty-vt over the ndterm core:
// a PTY runs $SHELL and its output is parsed into the cell grid, with keystrokes
// fed straight back to the PTY host-side.
function TerminalTab({ id, withMenu, onNewTab, onClose }: { id: number; withMenu: boolean; onNewTab: () => void; onClose: () => void }): React.ReactNode {
  return (
    <window
      title={id === 0 ? "Terminal" : `Terminal — ${id + 1}`}
      defaultWidth={860}
      defaultHeight={560}
      tabGroup="terminal"
      onNewTabRequested={onNewTab}
      onClosed={onClose}
    >
      {/* Process-wide app menu on the first open tab (same move as the
          browser example); File > Close from `defaults` closes a tab. */}
      {withMenu && (
        <menubar defaults>
          <menu label="File" testID="menu-file">
            <menuitem testID="menu-new-tab" label="New Tab" accelerator="primary+t" onSelect={onNewTab} />
          </menu>
        </menubar>
      )}
      <toolbarview>
        <headerbar title="Terminal" testID="chrome" />
        {/* No background or padding here on purpose: <terminal> owns its own
            surface, and a wrapper tinted a near-but-not-equal shade is what
            produced the mismatched frame this example used to show. */}
        <box
          orientation="vertical"
          style={{
            hexpand: true,
            vexpand: true,
            halign: "fill",
            valign: "fill",
          }}
        >
          <terminal
            cols={100}
            rows={30}
            fontSize={13}
            style={{ hexpand: true, vexpand: true }}
          />
        </box>
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
        <TerminalTab
          key={id}
          id={id}
          withMenu={i === 0}
          onNewTab={addTab}
          onClose={() => setTabs((open) => open.filter((t) => t !== id))}
        />
      ))}
    </>
  );
}

await render(<App />);
