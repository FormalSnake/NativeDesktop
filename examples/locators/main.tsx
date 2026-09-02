import { render, useState } from "@nativedesktop/react";

// Locator probe: the widgets scripts/locator-drive.ts needs to exercise the
// Playwright-shaped surface against a real host. A focusable field, a
// checkable box, a Select whose option titles reach the a11y probe, and a
// scroll view tall enough that the last row starts outside the clip, so
// scrollIntoViewIfNeeded has something to do.

const folders = ["Home", "Documents", "Downloads", "Pictures"];
const rows = Array.from({ length: 40 }, (_, i) => i);

function App(): React.ReactNode {
  const [query, setQuery] = useState("");
  const [notify, setNotify] = useState(false);
  const [folder, setFolder] = useState(0);

  return (
    <window title="ND Locators" defaultWidth={520} defaultHeight={420}>
      <box orientation="vertical" spacing={8} style={{ padding: 16 }}>
        <textinput
          testID="query-input"
          text={query}
          placeholder="Search notes"
          onChanged={(e) => setQuery(e.text)}
        />
        <label testID="query-label" text={`Query: ${query}`} />

        <checkbox
          testID="notify-check"
          label="Notify me"
          checked={notify}
          onToggled={(e) => setNotify(e.checked)}
        />
        <label testID="notify-label" text={`Notify: ${notify ? "on" : "off"}`} />

        <select
          testID="folder-select"
          options={folders}
          selectedIndex={folder}
          onSelectionChanged={(e) => setFolder(e.index)}
        />
        <label testID="folder-label" text={`Folder: ${folders[folder]}`} />

        <scrollview testID="row-scroll" minContentHeight={120} style={{ vexpand: true }}>
          <box orientation="vertical" spacing={4}>
            {rows.map((i) => (
              <label key={i} testID={`row-${i}`} text={`Row ${i}`} />
            ))}
          </box>
        </scrollview>
      </box>
    </window>
  );
}

render(<App />);
