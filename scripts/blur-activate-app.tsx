// Fixture for scripts/blur-activate-drive.ts: a search field and a text field
// beside a button, each field counting its `activate` events.
/** @jsxImportSource @nativedesktop/solid */
import { render } from "@nativedesktop/solid";
import { createSignal } from "solid-js";

function App() {
  const [search, setSearch] = createSignal(0);
  const [text, setText] = createSignal(0);
  return (
    <window title="Blur activate" defaultWidth={520} defaultHeight={240}>
      <box orientation="vertical" spacing={8}>
        <searchinput testID="search" placeholder="Search" onActivate={() => setSearch((n) => n + 1)} />
        <textinput testID="text" placeholder="Text" onActivate={() => setText((n) => n + 1)} />
        <button testID="button" label="Elsewhere" />
        <label testID="count" text={`search:${search()} text:${text()}`} />
      </box>
    </window>
  );
}

await render(() => <App />);
