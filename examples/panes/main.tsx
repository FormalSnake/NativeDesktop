// Split-pane example: a PaneTree of label leaves with split/close buttons,
// persisted through createStore. The store is loaded at top level, before
// render(), so the whole restore is synchronous plain code; structural
// changes flush() immediately, ratio drags ride the debounce (plus the
// store's exit hook when the host SIGTERMs the child).

import { createStore, render } from "@nativedesktop/react";
import { PaneTree, migratePanes, paneLeaves, samePaneShape, seedPanes, usePaneTree } from "@nativedesktop/panes";
import type { PaneModel } from "@nativedesktop/panes";

interface PaneData {
  label: string;
}

const store = createStore<PaneModel<PaneData>>({
  name: "panes",
  version: 1,
  defaults: seedPanes<PaneData>([{ label: "root" }]),
  // migratePanes self-heals garbage back to an empty model; seed a first pane
  // in that case so the window never comes up blank.
  migrate: (raw) => {
    const model = migratePanes<PaneData>(raw, (d): d is PaneData => typeof (d as PaneData)?.label === "string");
    return model.root ? model : seedPanes<PaneData>([{ label: "root" }]);
  },
  dir: process.env.ND_STORE_DIR,
});
const initial = await store.load();

function App(): React.ReactNode {
  const panes = usePaneTree<PaneData>(initial);

  const persist = (): void => {
    const next = panes.latest();
    const structural = !samePaneShape(store.get().root, next.root);
    store.set(next);
    if (structural) void store.flush();
  };

  const model = panes.model;
  const leaves = paneLeaves(model);
  const rootSplit = model.root?.kind === "split" ? model.root : undefined;

  return (
    <window title="ND Panes" defaultWidth={1000} defaultHeight={640}>
      <box orientation="vertical" spacing={6}>
        <box orientation="horizontal" spacing={6}>
          <label
            testID="panes-status"
            text={`panes:${leaves.length} ratio:${(rootSplit?.ratio ?? 0.5).toFixed(2)} focus:${model.focusedId}`}
          />
          <button
            testID="ratio-30"
            label="Root ratio 0.3"
            onClick={() => {
              if (!rootSplit) return;
              panes.setRatio(rootSplit.id, 0.3);
              persist();
            }}
          />
        </box>
        <PaneTree
          model={model}
          onChange={(next) => {
            panes.setModel(next);
            persist();
          }}
          testID="panes"
          renderLeaf={({ id, data, focused }) => (
            <box orientation="vertical" spacing={4}>
              <label testID={`pane-label-${id}`} text={`pane:${id}${focused ? " (focused)" : ""} ${data.label}`} />
              <box orientation="horizontal" spacing={4}>
                <button
                  testID={`split-h-${id}`}
                  label="Split H"
                  onClick={() => {
                    panes.split(id, "horizontal", { label: "split" });
                    persist();
                  }}
                />
                <button
                  testID={`split-v-${id}`}
                  label="Split V"
                  onClick={() => {
                    panes.split(id, "vertical", { label: "split" });
                    persist();
                  }}
                />
                <button
                  testID={`close-${id}`}
                  label="Close"
                  onClick={() => {
                    panes.close(id);
                    persist();
                  }}
                />
              </box>
            </box>
          )}
        />
      </box>
    </window>
  );
}

await render(<App />);
