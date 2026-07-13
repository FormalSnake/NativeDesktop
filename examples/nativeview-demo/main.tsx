import { render } from "@nativedesktop/react";

// A third-party native module hosting its OWN native widget. The generic
// <nativeview> widget routes to a factory that a dlopen'd plugin
// (plugins/colorview) registered under viewKind="colorview" via the v2 plugin
// ABI (register_view) — NO core schema or codegen edit was needed to add it,
// the same way you'd write an Expo/React Native native module. The module builds
// a GtkDrawingArea and fills it with the color from `props`.
function App(): React.ReactNode {
  return (
    <window title="Native Module Demo" defaultWidth={480} defaultHeight={320}>
      <nativeview
        viewKind="colorview"
        props={'{"color":"#3b82f6"}'}
        style={{ hexpand: true, vexpand: true }}
      />
    </window>
  );
}

await render(<App />);
