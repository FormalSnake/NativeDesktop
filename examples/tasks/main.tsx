import { render, useMemo, useState } from "@nativedesktop/react";

// ND Tasks — the "smallest real app": a single-pane task list, proving the
// M13 chrome machinery (menu bar + toolbar/headerbar) scales DOWN to a
// minimal shape, not just the three-pane Notes rework.
//
// Visual approach, mirroring examples/notes/main.tsx's doctrine:
//   - The window's one content child is a `<splitview>` holding exactly ONE
//     pane (`slot="content"`), not a bare `<toolbarview>` directly under
//     `<window>`. That's deliberate, not incidental: on the Mac backend a
//     `<toolbarview>` pane's headerbar/content only register with the
//     window's unified NSToolbar when the pane lands in a `<splitview>`
//     (swift/Sources/NDShell/HeaderBar.swift's `ndToolbarPaneAttachedToSplit`
//     fires from the SplitView structural arm only) — a toolbarview parented
//     straight under `<window>` would never attach there. GTK's
//     `<toolbarview>` is a real, self-sufficient AdwToolbarView and would
//     work standalone, but the single-`<splitview>`-pane wrapper keeps both
//     backends on the exact same proven path Notes already exercises, and
//     renders identically to a plain window (GTK: a sidebar-less
//     AdwOverlaySplitView; Mac: one NSSplitViewItem, no divider).
//   - `<menubar>` is a Window sibling of the splitview (app chrome, not
//     content): default menus (App/File/Edit/View/Window/Help) plus a
//     declared "Task" menu for New Task / Clear Completed.
//   - `cssClasses` reaches libadwaita's named classes (suggested-action,
//     dimmed, caption) exactly like Notes; `style` is kept only for geometry
//     (padding, hexpand/vexpand) — no color literals anywhere.
interface Task {
  id: number;
  title: string;
  done: boolean;
}

let nextId = 9;

const initialTasks: Task[] = [
  { id: 1, title: "Write project proposal", done: true },
  { id: 2, title: "Review open pull requests", done: false },
  { id: 3, title: "Update dependencies", done: false },
  { id: 4, title: "Fix sidebar layout bug", done: true },
  { id: 5, title: "Reply to design feedback", done: false },
  { id: 6, title: "Schedule team sync", done: false },
  { id: 7, title: "Draft release notes", done: false },
  { id: 8, title: "Back up database", done: true },
];

function App(): React.ReactNode {
  const [tasks, setTasks] = useState<Task[]>(initialTasks);
  const [query, setQuery] = useState("");

  // Search narrows the VISIBLE rows only — the progress fraction and the
  // "N of M done" caption always track the full task list, independent of
  // the filter (they answer "how much is done", not "how much is showing").
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (q === "") return tasks;
    return tasks.filter((t) => t.title.toLowerCase().includes(q));
  }, [tasks, query]);

  const total = tasks.length;
  const doneCount = tasks.filter((t) => t.done).length;
  const fraction = total === 0 ? 0 : doneCount / total;

  function addTask(): void {
    setTasks((prev) => [...prev, { id: nextId++, title: "New task", done: false }]);
  }

  function toggleTask(id: number, done: boolean): void {
    setTasks((prev) => prev.map((t) => (t.id === id ? { ...t, done } : t)));
  }

  function clearCompleted(): void {
    setTasks((prev) => prev.filter((t) => !t.done));
  }

  return (
    <window title="Tasks" defaultWidth={480} defaultHeight={640}>
      <menubar testID="tasks-menubar">
        <menu label="Task" testID="task-menu">
          <menuitem
            testID="menu-new-task"
            label="New Task"
            iconName="list-add"
            accelerator="primary+n"
            onSelect={addTask}
          />
          <menuitem
            testID="menu-clear-completed"
            label="Clear Completed"
            accelerator="primary+shift+k"
            onSelect={clearCompleted}
          />
        </menu>
      </menubar>
      <splitview testID="tasks-split">
        <toolbarview slot="content" testID="tasks-toolbar">
          <headerbar title="Tasks" testID="tasks-header">
            <button
              testID="new-task-button"
              label=""
              iconName="list-add"
              onClick={addTask}
              slot="end"
              cssClasses={["suggested-action"]}
            />
          </headerbar>
          <box
            orientation="vertical"
            spacing={12}
            cssClasses={["view"]}
            style={{ hexpand: true, vexpand: true, padding: 16 }}
          >
            <searchinput
              testID="task-search"
              text={query}
              placeholder="Filter tasks"
              onChanged={(e) => setQuery(e.text)}
            />
            {/* vexpand: the row list fills the remaining pane height. */}
            <scrollview testID="task-scroll" minContentHeight={360} style={{ vexpand: true }}>
              <box orientation="vertical" spacing={2}>
                {filtered.map((t) => (
                  <box
                    key={t.id}
                    orientation="horizontal"
                    style={{ padding: { top: 6, bottom: 6, left: 4, right: 4 } }}
                  >
                    <checkbox
                      testID={`task-row-${t.id}`}
                      label={t.title}
                      checked={t.done}
                      onToggled={(e) => toggleTask(t.id, e.checked)}
                    />
                  </box>
                ))}
              </box>
            </scrollview>
            <separator orientation="horizontal" />
            <progressbar testID="task-progress" fraction={fraction} />
            <label
              testID="task-count"
              text={`${doneCount} of ${total} done`}
              cssClasses={["dimmed", "caption"]}
            />
          </box>
        </toolbarview>
      </splitview>
    </window>
  );
}

await render(<App />);
