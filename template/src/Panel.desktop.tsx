// A `.desktop.tsx` component, the NativeDesktop mirror of React Native's
// `.native.tsx`. It is an ordinary `.tsx`, so `import { Panel } from
// "./Panel.desktop"` resolves with no extension. Desktop UI lives here,
// apart from shared code such as the toggle in hooks/useToggle.ts; it renders
// to native widgets, not the DOM.
import { useToggle } from "./hooks/useToggle.ts";

export function Panel() {
  const [open, toggle] = useToggle();
  return (
    <box orientation="vertical" spacing={8}>
      <label testID="panel-status" text={open() ? "Panel: open" : "Panel: closed"} />
      <button testID="panel-toggle" label="Toggle panel" onClick={toggle} />
    </box>
  );
}
