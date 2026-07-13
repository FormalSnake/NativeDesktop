import { render } from "@nativedesktop/react";

// A real terminal emulator built with the framework: the <terminal> widget hosts
// a native drawing surface (GtkDrawingArea on GTK, NDTerminalView/CoreText on
// AppKit) driven by libghostty-vt over the ndterm core — a PTY runs $SHELL, its
// output is parsed into the terminal grid, and keystrokes are fed straight back
// to the PTY host-side. Phase A: the widget owns its own PTY + input; no props
// beyond the shell/geometry are required.
function App(): React.ReactNode {
  return (
    <window title="NativeDesktop Terminal" defaultWidth={720} defaultHeight={460}>
      <terminal cols={80} rows={24} fontSize={13} />
    </window>
  );
}

await render(<App />);
