import { render, sendCommand, useRef, useState } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

// nd-widgets probe (2026-07-17 wave): a deterministic assertion target for
// three new capabilities — C3 icon+label/icon-only button parity, C4
// onHoverChanged (button + box), C5 the Window "present" command — on both
// backends. Two always-mounted windows (peer of examples/multiwindow) so
// "present" has something to bring back to front.

function App(): React.ReactNode {
  const winB = useRef<NdNodeRef<"window">>(null);
  const [buttonHover, setButtonHover] = useState(false);
  const [boxHover, setBoxHover] = useState(false);
  const [selectedRun, setSelectedRun] = useState("a1");

  return (
    <>
      <window title="ND Widgets Probe A" defaultWidth={480} defaultHeight={560}>
        <box orientation="vertical" spacing={12} style={{ padding: 16 }}>
          <box orientation="horizontal" spacing={8}>
            {/* C3: iconName + label="" renders icon-only; iconName + non-empty label renders icon AND text. */}
            <button testID="icon-only-btn" label="" iconName="view-refresh-symbolic" onClick={() => {}} />
            <button testID="icon-label-btn" label="Refresh" iconName="view-refresh-symbolic" onClick={() => {}} />
          </box>

          {/* C4: onHoverChanged on button. */}
          <button testID="hover-btn" label="Hover me" onHoverChanged={(e) => setButtonHover(e.checked)} />
          <label testID="hover-btn-label" text={`Button hover: ${buttonHover ? "yes" : "no"}`} />

          {/* C4: onHoverChanged on box. */}
          <box testID="hover-box" orientation="vertical" spacing={4} style={{ padding: 12 }} onHoverChanged={(e) => setBoxHover(e.checked)}>
            <label text="Hover zone (box)" />
          </box>
          <label testID="hover-box-label" text={`Box hover: ${boxHover ? "yes" : "no"}`} />

          {/* C5: window "present" — raise/focus Window B from Window A. */}
          <button
            testID="present-b-btn"
            label="Present Window B"
            onClick={() => { if (winB.current) sendCommand(winB.current, "present"); }}
          />

          {/* AppKit navigation-sidebar reach check: row buttons nested two
              levels below the classed box (a host section wrapping each run
              row), the app's real Sidebar.tsx shape — must still become
              native source-list table rows, not an empty table. */}
          <box cssClasses={["nd-native-sidebar"]} style={{ vexpand: true }} testID="sidebar-probe">
            <box orientation="vertical" spacing={4} testID="host-a">
              <label text="Host A" cssClasses={["heading"]} />
              <button
                testID="run-a1"
                label="Run one"
                iconName="media-playback-start-symbolic"
                labelAlign="start"
                onClick={() => setSelectedRun("a1")}
                cssClasses={selectedRun === "a1" ? ["flat", "suggested-action"] : ["flat"]}
              />
              <button
                testID="run-a2"
                label="Run two"
                iconName="media-playback-start-symbolic"
                labelAlign="start"
                onClick={() => setSelectedRun("a2")}
                cssClasses={selectedRun === "a2" ? ["flat", "suggested-action"] : ["flat"]}
              />
            </box>
            <box orientation="vertical" spacing={4} testID="host-b">
              <label text="Host B" cssClasses={["heading"]} />
              <button
                testID="run-b1"
                label="Run three"
                iconName="media-playback-start-symbolic"
                labelAlign="start"
                onClick={() => setSelectedRun("b1")}
                cssClasses={selectedRun === "b1" ? ["flat", "suggested-action"] : ["flat"]}
              />
            </box>
          </box>
        </box>
      </window>

      <window ref={winB} title="ND Widgets Probe B" defaultWidth={360} defaultHeight={220}>
        <box orientation="vertical" spacing={8} style={{ padding: 16 }}>
          <label testID="window-b-label" text="Window B — bring me to front" />
        </box>
      </window>
    </>
  );
}

await render(<App />);
