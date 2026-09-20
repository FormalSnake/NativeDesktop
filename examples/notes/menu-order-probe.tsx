import { render, useState } from "@nativedesktop/react";

// NOT imported by main.tsx. The acceptance fixture for menu child ORDER,
// driven headlessly by scripts/menu-order-drive.ts on BOTH backends. One
// keyed list is rendered twice, under a <menubutton> and under a menubar
// <menu>, and three buttons mutate it the three ways React expresses:
//
//   reorder      moves an existing child (a bare insertBefore, no remove)
//   remove       drops one from the middle
//   insert       adds one in the middle
//
// The label mirrors the React order so a drive can compare it against the
// native model the menuModel RPC reads back.
const INITIAL = ["Alpha", "Bravo", "Charlie", "Delta"];

function App(): React.ReactNode {
  const [items, setItems] = useState(INITIAL);
  return (
    <window title="ND Menu Order Probe" defaultWidth={520} defaultHeight={320}>
      <menubar defaults={false} testID="probe-menubar">
        <menu label="Tabs" testID="probe-tabs-menu">
          {items.map((name) => (
            <menuitem key={name} testID={`bar-${name}`} label={name} />
          ))}
        </menu>
      </menubar>
      <box orientation="vertical" spacing={8} style={{ padding: 16 }}>
        <menubutton testID="probe-owner" label="Owner">
          {items.map((name) => (
            <menuitem key={name} testID={`own-${name}`} label={name} />
          ))}
          <menuitem role="separator" />
          <menu label="More">
            <menuitem label="Nested one" />
            <menuitem label="Nested two" />
          </menu>
        </menubutton>
        <label testID="probe-order" text={`order=${items.join("|")}`} />
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

await render(<App />);
