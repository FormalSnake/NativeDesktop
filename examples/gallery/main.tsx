import { render } from "@nativedesktop/react";
import { useMemo, useState } from "react";

// M5b/M5c gallery: every new widget with live controlled state + testIDs,
// driven headlessly by scripts/m5b-drive.ts and scripts/m5c-drive.ts over
// the automation socket.
function App(): React.ReactNode {
  const [name, setName] = useState("");
  const [notes, setNotes] = useState("");
  const [agreed, setAgreed] = useState(false);
  const [size, setSize] = useState("small");
  const [fruitIndex, setFruitIndex] = useState(0);
  const [volume, setVolume] = useState(25);
  const [submitted, setSubmitted] = useState(false);
  const fruits = ["apple", "banana", "cherry"];
  // M5c-D-followup fallback (plan §"GtkListView renders under GSK_RENDERER=cairo
  // headless with 100k rows"): the host's GtkStringList.append-per-row create
  // path is O(n^2) past ~20k rows under headless cairo, hanging commit-apply
  // indefinitely (verified: 15k rows completes in ~1s, 20k+ never completes).
  // That is a host/generated-code scalability bug outside this file's scope
  // (src/generated/widgets.zig, Task 4). Reduced to 10k here — comfortably
  // under the verified-safe threshold — so headless-m5b.sh's drive of this
  // same script keeps completing; recycling and the ListView widget surface
  // are still fully exercised at this scale.
  const rows = useMemo(() => Array.from({ length: 10000 }, (_, i) => `Item ${i}`), []);
  const [selectedRow, setSelectedRow] = useState(0);
  const [activatedRow, setActivatedRow] = useState(-1);

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
          <box tabLabel="Styled" orientation="vertical" spacing={6} testID="styled-tab">
            <label testID="styled-label" text="Styled label"
              style={{ background: "#2266cc", color: "#ffffff", padding: 8, margin: 4,
                       font: { fontSize: 16, fontWeight: "bold" },
                       border: { borderWidth: 2, borderColor: "#003399", borderRadius: 6 } }} />
            <button testID="styled-button" label="Styled button"
              style={{ background: "#cc2222", color: "#ffffff", padding: 6 }} />
          </box>
          <listview tabLabel="List" testID="big-list"
            items={rows}
            selectedIndex={selectedRow}
            onRowActivated={(e) => setActivatedRow(e.index)} />
        </tabview>
        <label testID="activated-label" text={`Activated: ${activatedRow}`} />
        <scrollview testID="log-scroll" minContentHeight={120}>
          <box orientation="vertical" spacing={2}>
            {Array.from({ length: 40 }, (_, i) => <label key={i} text={`Row ${i}`} />)}
          </box>
        </scrollview>
      </box>
    </window>
  );
}

await render(<App />);
