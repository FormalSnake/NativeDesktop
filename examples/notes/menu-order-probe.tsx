import { render } from "@nativedesktop/solid";
import { For, createSignal } from "solid-js";

// NOT imported by main.tsx. The acceptance fixture for menu child ORDER,
// driven headlessly by scripts/menu-order-drive.ts on BOTH backends. One
// keyed list is rendered twice, under a <menubutton> and under a menubar
// <menu>, and three buttons mutate it the three ways a keyed list expresses:
//
//   reorder      moves an existing child (a bare insertBefore, no remove)
//   remove       drops one from the middle
//   insert       adds one in the middle
//
// The label mirrors the list order so a drive can compare it against the
// native model the menuModel RPC reads back.
const INITIAL = ["Alpha", "Bravo", "Charlie", "Delta"];

function App() {
  const [items, setItems] = createSignal(INITIAL);
  return (
    <window title="ND Menu Order Probe" defaultWidth={520} defaultHeight={320}>
      <menubar testID="probe-menubar">
        <menu label="Tabs" testID="probe-tabs-menu">
          <For each={items()}>{(name) => <menuitem testID={`bar-${name}`} label={name} />}</For>
        </menu>
      </menubar>
      <box orientation="vertical" spacing={8} style={{ padding: 16 }}>
        <menubutton testID="probe-owner" label="Owner">
          <For each={items()}>{(name) => <menuitem testID={`own-${name}`} label={name} />}</For>
          <menuitem role="separator" />
          <menu label="More">
            <menuitem label="Nested one" />
            <menuitem label="Nested two" />
          </menu>
        </menubutton>
        <label testID="probe-order" text={`order=${items().join("|")}`} />
        <button
          testID="probe-reorder"
          label="Reorder"
          onClick={() => setItems((xs) => [xs[1]!, xs[2]!, xs[0]!, ...xs.slice(3)])}
        />
        <button
          testID="probe-remove"
          label="Remove middle"
          onClick={() => setItems((xs) => xs.filter((_, i) => i !== 1))}
        />
        <button
          testID="probe-insert"
          label="Insert middle"
          onClick={() => setItems((xs) => [...xs.slice(0, 1), "Echo", ...xs.slice(1)])}
        />
      </box>
    </window>
  );
}

await render(() => <App />);
