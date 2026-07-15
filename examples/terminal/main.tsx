import { render } from "@nativedesktop/react";

// A terminal app with REAL native chrome. The <toolbarview> + <headerbar> pair
// sits directly under <window>, so the header lands in the platform's real
// titlebar, exactly like the browser example: a unified NSToolbar with the
// traffic lights inline on macOS, a real AdwHeaderBar (window controls
// included) on GTK. Below it, a dark padded frame holds the terminal so the
// emulator reads as an intentional app surface, Ghostty-style, rather than a
// bare grid.
//
// The <terminal> widget hosts a native drawing surface (GtkDrawingArea on GTK,
// NDTerminalView/CoreText on AppKit) driven by libghostty-vt over the ndterm core:
// a PTY runs $SHELL and its output is parsed into the cell grid, with keystrokes
// fed straight back to the PTY host-side.
function App(): React.ReactNode {
  return (
    <window title="Terminal" defaultWidth={860} defaultHeight={560}>
      <toolbarview>
        <headerbar title="Terminal" testID="chrome" />
        <box
          orientation="vertical"
          style={{
            background: "#0e0e12",
            padding: 14,
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

await render(<App />);
