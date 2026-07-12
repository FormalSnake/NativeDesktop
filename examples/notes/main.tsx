import { render, useMemo, useState } from "@nativedesktop/react";

// ND Notes — a note-taking app used as a framework-suitability stress test
// (see scripts/notes-drive.ts for the headless proof). All state is in
// memory; there is no persistence layer by design.
//
// Visual approach: native chrome, not hand-rolled facsimiles of it.
//   - The `<window>` is edge-to-edge (AdwApplicationWindow): the `<splitview>`
//     fills to the very top, GNOME-style. Each pane is wrapped in a
//     `<toolbarview>` (AdwToolbarView) whose first child is a `<headerbar>`
//     — so the sidebar and the content pane each carry their OWN header bar
//     at the top, instead of one shared window titlebar.
//   - `<splitview>` is the real sidebar/content split (AdwOverlaySplitView
//     on GTK, NSSplitView + vibrancy sidebar on Mac) — no hand-rolled
//     two-Box row with a hardcoded sidebar background.
//   - `cssClasses` reaches libadwaita's named classes (navigation-sidebar,
//     suggested-action, destructive-action, pill, dimmed, caption, view,
//     flat). libadwaita is initialized host-side (src/gtk/main.zig calls
//     adw.init()), so AdwStyleManager tracks the system color scheme — the
//     whole app follows light/dark automatically, with no per-widget color
//     values.
//   - `style` is kept only for theme-neutral geometry (padding, font size)
//     and layout (hexpand/vexpand/halign/valign — GTK widget properties
//     that drive the layout engine) plus the one deliberate color literal
//     below (the pinned-row accent border, chosen to read on both themes).
// No background/color literals anywhere else: dark mode is automatic on
// both platforms.

interface Note {
  id: number;
  title: string;
  body: string;
  pinned: boolean;
}

let nextId = 3;

const initialNotes: Note[] = [
  { id: 1, title: "Welcome to ND Notes", body: "This is your first note. Select it, edit it, or create a new one.", pinned: true },
  { id: 2, title: "Shopping list", body: "Milk\nEggs\nBread", pinned: false },
];

function wordCount(text: string): number {
  const trimmed = text.trim();
  return trimmed.length === 0 ? 0 : trimmed.split(/\s+/).length;
}

function sortNotes(notes: Note[]): Note[] {
  return [...notes].sort((a, b) => {
    if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
    return b.id - a.id;
  });
}

function App(): React.ReactNode {
  const [notes, setNotes] = useState<Note[]>(initialNotes);
  const [selectedId, setSelectedId] = useState<number | null>(1);
  const [query, setQuery] = useState("");
  const [savedPulse, setSavedPulse] = useState(true);

  const sorted = useMemo(() => sortNotes(notes), [notes]);
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (q === "") return sorted;
    return sorted.filter((n) => n.title.toLowerCase().includes(q) || n.body.toLowerCase().includes(q));
  }, [sorted, query]);

  const selected = notes.find((n) => n.id === selectedId) ?? null;

  function updateSelected(patch: Partial<Note>): void {
    if (selected == null) return;
    setNotes((prev) => prev.map((n) => (n.id === selected.id ? { ...n, ...patch } : n)));
    setSavedPulse(true);
  }

  function createNote(): void {
    const note: Note = { id: nextId++, title: "Untitled note", body: "", pinned: false };
    setNotes((prev) => [...prev, note]);
    setSelectedId(note.id);
    setQuery("");
  }

  function deleteSelected(): void {
    if (selected == null) return;
    const remaining = notes.filter((n) => n.id !== selected.id);
    setNotes(remaining);
    const nextSorted = sortNotes(remaining);
    setSelectedId(nextSorted.length > 0 ? nextSorted[0]!.id : null);
  }

  return (
    <window title="ND Notes" defaultWidth={900} defaultHeight={600}>
      <splitview sidebarWidth={0.28} testID="split">
        <toolbarview slot="sidebar" testID="sidebar-toolbar">
          <headerbar testID="sidebar-header">
            <button
              testID="new-note-button"
              label="＋ New Note"
              onClick={createNote}
              slot="start"
              cssClasses={["suggested-action"]}
            />
          </headerbar>
          <box
            orientation="vertical"
            spacing={8}
            cssClasses={["navigation-sidebar"]}
            style={{ vexpand: true, padding: { top: 12, bottom: 12, left: 10, right: 10 } }}
          >
          <textinput
            testID="search-input"
            text={query}
            placeholder="Search notes"
            onChanged={(e) => setQuery(e.text)}
          />
          {/* vexpand: the list fills the sidebar's remaining height. */}
          <scrollview testID="note-list-scroll" minContentHeight={380} style={{ vexpand: true }}>
            <box orientation="vertical" spacing={3}>
              {filtered.map((n) => {
                const isSelected = n.id === selectedId;
                // Button.label is create-only (docs/widgets.md) — it cannot
                // be updated in place once mounted. Selection uses the
                // theme's suggested-action (accent) class; a plain flat row
                // otherwise. Pin state is a left accent border via `style`
                // (which DOES update live) — the one deliberate color
                // literal, an amber that reads on both light and dark.
                return (
                  <button
                    key={`${n.id}:${n.title}`}
                    testID={`note-row-${n.id}`}
                    label={n.title || "Untitled note"}
                    onClick={() => setSelectedId(n.id)}
                    cssClasses={isSelected ? ["suggested-action"] : ["flat"]}
                    style={{
                      padding: { top: 8, bottom: 8, left: 10, right: 10 },
                      halign: "fill",
                      border: n.pinned ? { borderWidth: 3, borderColor: "#e5a50a", borderRadius: 8 } : { borderRadius: 8 },
                    }}
                  />
                );
              })}
            </box>
          </scrollview>
            <label
              testID="note-count-label"
              text={`${filtered.length} of ${notes.length} note${notes.length === 1 ? "" : "s"}`}
              cssClasses={["dimmed", "caption"]}
            />
          </box>
        </toolbarview>

        <toolbarview slot="content" testID="content-toolbar">
          {/* HeaderBar.title is create-only (docs/widgets.md); key on the
              displayed value so the header remounts when the title changes. */}
          <headerbar
            key={selected != null ? `${selected.id}:${selected.title}` : "empty"}
            testID="content-header"
            title={selected != null ? selected.title || "Untitled note" : "ND Notes"}
          />
          {/* hexpand+vexpand: the content pane claims all space the sidebar doesn't. */}
          <box orientation="vertical" spacing={12} cssClasses={["view"]} style={{ hexpand: true, vexpand: true, padding: 20 }}>
            {selected != null ? (
            // key={selected.id}: prop `update` ops only reach GTK for
            // style/testID/label-text (src/tree.zig's update handler
            // resolves the widget kind from the wire message, which is
            // never populated for "update" ops — only "create" carries a
            // widget kind, per packages/react/src/host-config.ts's
            // commitUpdate). A same-widget-instance prop push (typing in
            // THIS textinput) is invisible to that gap since the GTK widget
            // already holds the value the user just typed; a cross-widget
            // push (switching to a DIFFERENT note) is not, and silently
            // no-ops without a remount. Keying on the note id forces create
            // (not update) on note switch, sidestepping the gap; framework
            // bug, not patched here.
            <box key={selected.id} orientation="vertical" spacing={12} style={{ vexpand: true }}>
              <textinput
                testID="title-input"
                text={selected.title}
                placeholder="Title"
                onChanged={(e) => updateSelected({ title: e.text })}
                style={{ font: { fontSize: 20, fontWeight: "bold" }, padding: 4 }}
              />
              <box orientation="horizontal" spacing={10}>
                <checkbox
                  testID="pin-checkbox"
                  label="Pinned"
                  checked={selected.pinned}
                  onToggled={(e) => updateSelected({ pinned: e.checked })}
                />
                <button
                  testID="delete-note-button"
                  label="Delete"
                  onClick={deleteSelected}
                  cssClasses={["destructive-action", "pill"]}
                />
              </box>
              <separator orientation="horizontal" />
              {/* TextArea has its own minContentHeight floor (Task 4) — no
                  ScrollView wrapper needed; vexpand lets it fill the pane. */}
              <textarea
                testID="editor-textarea"
                minContentHeight={320}
                text={selected.body}
                onChanged={(e) => updateSelected({ body: e.text })}
                style={{ font: { fontSize: 14 }, padding: 4, vexpand: true }}
              />
              <label
                testID="status-label"
                text={`${wordCount(selected.body)} word${wordCount(selected.body) === 1 ? "" : "s"} · ${savedPulse ? "Saved" : ""}`}
                cssClasses={["dimmed", "caption"]}
              />
            </box>
          ) : (
            <label testID="empty-state-label" text="No note selected. Create one to get started." cssClasses={["dimmed"]} />
          )}
          </box>
        </toolbarview>
      </splitview>
    </window>
  );
}

await render(<App />);
