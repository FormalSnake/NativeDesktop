import { render, useMemo, useState } from "@nativedesktop/react";

// ND Notes — a note-taking app used as a framework-suitability stress test
// (see scripts/notes-drive.ts for the headless proof). All state is in
// memory; there is no persistence layer by design.
//
// Visual target: GNOME/libadwaita's look (AdwNavigationSplitView sidebar +
// content, AdwHeaderBar, flat/pill/suggested-action/destructive-action
// buttons, .dim-label secondary text). NONE of that exists as a widget or a
// style-class hook here yet (see FRAMEWORK SUITABILITY report) — this file
// is plain GTK4 (no libadwaita loaded, confirmed: no Adwaita import anywhere
// under src/gtk/), and `style` (docs/styling.md) has no `class`/`cssClasses`
// escape hatch to reach GTK's or Adwaita's named CSS classes; every visual
// below is hand-rolled from the property-only style subset (background/
// color/font/padding/margin/border). Palette approximates libadwaita's
// default light variant (window #fafafa, sidebar #ebebeb, accent #3584e4).

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
      {/* Would be AdwToolbarView/AdwHeaderBar + AdwNavigationSplitView on a
          real GNOME app; there is no header-bar or split-view widget here,
          so the "header" below is just a styled Box row and the sidebar is
          a plain Box sized by its content (no true split-view divider drag). */}
      <box orientation="vertical" spacing={0} style={{ background: "#fafafa" }}>
        <box
          orientation="horizontal"
          spacing={8}
          style={{ background: "#ebebeb", padding: { top: 10, bottom: 10, left: 12, right: 12 },
                   border: { borderWidth: 1, borderColor: "#d8d8d8" } }}
        >
          <label
            testID="app-title"
            text="ND Notes"
            style={{ font: { fontSize: 15, fontWeight: "bold" }, color: "#1a1a1a" }}
          />
        </box>
        <box orientation="horizontal" spacing={0}>
          <box
            orientation="vertical"
            spacing={8}
            style={{ background: "#ebebeb", padding: { top: 12, bottom: 12, left: 10, right: 10 } }}
          >
            <button
              testID="new-note-button"
              label="＋ New Note"
              onClick={createNote}
              style={{ background: "#3584e4", color: "#ffffff", padding: 8, border: { borderRadius: 16 } }}
            />
            <textinput
              testID="search-input"
              text={query}
              placeholder="Search notes"
              onChanged={(e) => setQuery(e.text)}
              style={{ padding: 6, border: { borderRadius: 8 }, background: "#ffffff", color: "#1a1a1a" }}
            />
            <scrollview testID="note-list-scroll" minContentHeight={380}>
              {/* key=orderSignature: React's insertBefore/append for a Box's
                  children, when several siblings are reordered/inserted in
                  the SAME commit, can produce the wrong final GTK order —
                  the host's insertBefore (src/generated/widgets.zig) does
                  `insertChildAfter(box, child, getPrevSibling(before))`,
                  recomputed per call, so two inserts anchored at the same
                  `before` land adjacent in REVERSE call order rather than
                  each other's intended relative order; separately, GTK's
                  `gtk_box_insert_child_after` requires the moved child's
                  parent to be NULL, so React repositioning an
                  ALREADY-mounted child via insertBefore hits a `Gtk-CRITICAL`
                  assertion and silently fails to move at all (verified live:
                  scripts/notes-drive.ts's pin-then-assert-reorder step
                  reproduced both). Keying the whole row list on a signature
                  of the fully sorted id order forces a clean remount (full
                  create+append in final order) instead of a partial
                  insert/move sequence whenever pinning (or any other sort-
                  affecting change) reorders the list — framework bug, not
                  patched here; see FRAMEWORK SUITABILITY report. */}
              <box key={filtered.map((n) => n.id).join(",")} orientation="vertical" spacing={3}>
                {filtered.map((n) => {
                  const isSelected = n.id === selectedId;
                  // Button.label is create-only (docs/widgets.md) — it
                  // cannot be updated in place once mounted. Pin state is
                  // conveyed via `style` instead (a left accent border),
                  // which DOES update live.
                  return (
                    <button
                      key={`${n.id}:${n.title}`}
                      testID={`note-row-${n.id}`}
                      label={n.title || "Untitled note"}
                      onClick={() => setSelectedId(n.id)}
                      style={
                        isSelected
                          ? { background: "#3584e4", color: "#ffffff", padding: { top: 8, bottom: 8, left: 10, right: 10 },
                              border: { borderRadius: 8, borderWidth: n.pinned ? 3 : 0, borderColor: "#f5c211" } }
                          : { background: "#fafafa", color: "#1a1a1a", padding: { top: 8, bottom: 8, left: 10, right: 10 },
                              border: { borderRadius: 8, borderWidth: n.pinned ? 3 : 0, borderColor: "#f5c211" } }
                      }
                    />
                  );
                })}
              </box>
            </scrollview>
            <label
              testID="note-count-label"
              text={`${filtered.length} of ${notes.length} note${notes.length === 1 ? "" : "s"}`}
              style={{ color: "#8a8a8a", font: { fontSize: 12 } }}
            />
          </box>

          <box orientation="vertical" spacing={12} style={{ background: "#ffffff", padding: 20 }}>
            {selected != null ? (
              // key={selected.id}: prop `update` ops only reach GTK for
              // style/testID/label-text (src/tree.zig's update handler
              // resolves the widget kind from the wire message, which is
              // never populated for "update" ops — only "create" carries a
              // widget kind, per packages/react/src/host-config.ts's
              // commitUpdate). A same-widget-instance prop push (typing in
              // THIS textinput) is invisible to that gap since the GTK
              // widget already holds the value the user just typed; a
              // cross-widget push (switching to a DIFFERENT note) is not,
              // and silently no-ops without a remount. Keying on the note
              // id forces create (not update) on note switch, sidestepping
              // the gap; framework bug, not patched here.
              <box key={selected.id} orientation="vertical" spacing={12}>
                <textinput
                  testID="title-input"
                  text={selected.title}
                  placeholder="Title"
                  onChanged={(e) => updateSelected({ title: e.text })}
                  style={{ font: { fontSize: 20, fontWeight: "bold" }, padding: 4, color: "#1a1a1a" }}
                />
                <box orientation="horizontal" spacing={10}>
                  <checkbox
                    testID="pin-checkbox"
                    label="Pinned"
                    checked={selected.pinned}
                    onToggled={(e) => updateSelected({ pinned: e.checked })}
                    style={{ color: "#1a1a1a" }}
                  />
                  <button
                    testID="delete-note-button"
                    label="Delete"
                    onClick={deleteSelected}
                    style={{ background: "#e01b24", color: "#ffffff", padding: { top: 6, bottom: 6, left: 12, right: 12 }, border: { borderRadius: 16 } }}
                  />
                </box>
                <separator orientation="horizontal" />
                {/* TextArea alone has no size floor and can collapse to 0
                    height with empty content (degenerate bounds -> not
                    actionable); ScrollView's minContentHeight gives the
                    editor a floor and doubles as scroll for long notes. */}
                <scrollview testID="editor-scroll" minContentHeight={320}>
                  <textarea
                    testID="editor-textarea"
                    text={selected.body}
                    onChanged={(e) => updateSelected({ body: e.text })}
                    style={{ font: { fontSize: 14 }, padding: 4, background: "#ffffff", color: "#1a1a1a" }}
                  />
                </scrollview>
                <label
                  testID="status-label"
                  text={`${wordCount(selected.body)} word${wordCount(selected.body) === 1 ? "" : "s"} · ${savedPulse ? "Saved" : ""}`}
                  style={{ color: "#8a8a8a", font: { fontSize: 12 } }}
                />
              </box>
            ) : (
              <label testID="empty-state-label" text="No note selected. Create one to get started." style={{ color: "#8a8a8a" }} />
            )}
          </box>
        </box>
      </box>
    </window>
  );
}

await render(<App />);
