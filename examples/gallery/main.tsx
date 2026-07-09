import { render } from "@nativedesktop/react";
import { createElement, useState } from "react";

// M5b gallery: every new widget with live controlled state + testIDs, driven
// headlessly by scripts/m5b-drive.ts over the automation socket.
function App(): React.ReactNode {
  const [name, setName] = useState("");
  const [notes, setNotes] = useState("");
  const [agreed, setAgreed] = useState(false);
  const [size, setSize] = useState("small");
  const [fruitIndex, setFruitIndex] = useState(0);
  const [volume, setVolume] = useState(25);
  const [submitted, setSubmitted] = useState(false);
  const fruits = ["apple", "banana", "cherry"];

  return (
    <window title="NativeDesktop M5b Gallery" defaultWidth={560} defaultHeight={680}>
      <box orientation="vertical" spacing={8}>
        <tabview testID="gallery-tabs">
          <box tabLabel="Form" orientation="vertical" spacing={6}>
            <textinput
              testID="name-input"
              text={name}
              placeholder="Your name"
              onChanged={(e) => setName(e.text)}
              onActivate={() => setSubmitted(true)}
            />
            <label testID="echo-label" text={`Echo: ${name}`} />
            <label testID="submit-label" text={`Submitted: ${submitted ? "yes" : "no"}`} />
            <separator orientation="horizontal" />
            <textarea testID="notes-area" text={notes} onChanged={(e) => setNotes(e.text)} />
            <label testID="notes-label" text={`Notes: ${notes}`} />
            <checkbox testID="agree-check" label="I agree" checked={agreed} onToggled={(e) => setAgreed(e.checked)} />
            <label testID="agree-label" text={`Agreed: ${agreed ? "yes" : "no"}`} />
            <radio testID="size-small" group="size" label="Small" checked={size === "small"}
              onToggled={(e) => { if (e.checked) setSize("small"); }} />
            <radio testID="size-large" group="size" label="Large" checked={size === "large"}
              onToggled={(e) => { if (e.checked) setSize("large"); }} />
            <label testID="size-label" text={`Size: ${size}`} />
            <select testID="fruit-select" options={fruits} selectedIndex={fruitIndex}
              onSelectionChanged={(e) => setFruitIndex(e.index)} />
            <label testID="fruit-label" text={`Fruit: ${fruits[fruitIndex]}`} />
            <slider testID="volume-slider" min={0} max={100} step={1} value={volume}
              onValueChanged={(e) => setVolume(e.value)} />
            <progressbar testID="volume-progress" fraction={volume / 100} />
            <label testID="volume-label" text={`Volume: ${volume}`} />
            <box orientation="horizontal" spacing={6}>
              <spinner testID="busy-spinner" spinning={true} />
              <image testID="smile-icon" iconName="face-smile-symbolic" />
              <webview testID="web-stub" />
            </box>
          </box>
          <grid tabLabel="Grid" testID="layout-grid">
            <label gridRow={0} gridColumn={0} text="r0c0" />
            <label gridRow={0} gridColumn={1} text="r0c1" />
            <label gridRow={1} gridColumn={0} gridColumnSpan={2} text="r1 span2" />
          </grid>
        </tabview>
        <scrollview testID="log-scroll" minContentHeight={120}>
          <box orientation="vertical" spacing={2}>
            {Array.from({ length: 40 }, (_, i) => createElement("label", { key: i, text: `Row ${i}` }))}
          </box>
        </scrollview>
      </box>
    </window>
  );
}

await render(<App />);
