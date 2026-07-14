import { defineNativeComponent, render, useRef, useState, type NativeComponentRef } from "@nativedesktop/react";

interface ColorProps { color: string }
interface ColorEvent { source: "gtk" | "appkit" }
type ColorCommand = Record<string, never>;

const ColorView = defineNativeComponent<ColorProps, ColorEvent, ColorCommand>({ viewKind: "app.colorview" });

function App(): React.ReactNode {
  const [color, setColor] = useState("#3b82f6");
  const [lastSource, setLastSource] = useState("none");
  const native = useRef<NativeComponentRef>(null);
  return (
    <window title="App-owned Native Component" defaultWidth={480} defaultHeight={360}>
      <box orientation="vertical" spacing={12} style={{ padding: 16 }}>
        <ColorView
          ref={native}
          props={{ color }}
          onNativeEvent={({ name, data }) => {
            if (name === "pressed") setLastSource(data.source);
          }}
          style={{ hexpand: true, vexpand: true }}
        />
        <label text={`Native event source: ${lastSource}`} />
        <box orientation="horizontal" spacing={8}>
          <button label="Change color" onClick={() => setColor(color === "#3b82f6" ? "#ef4444" : "#3b82f6")} />
          <button label="Reset natively" onClick={() => native.current?.send("reset", {})} />
        </box>
      </box>
    </window>
  );
}

await render(<App />);
