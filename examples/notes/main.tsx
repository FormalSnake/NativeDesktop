import { render } from "@nativedesktop/react";
import { For, Show, createMemo, createSignal, createStore } from "solid-js";

// ND Notes — a note-taking app used as a framework-suitability stress test
// (see scripts/notes-drive.ts for the headless proof). All state is in
// memory; there is no persistence layer by design.
//
// Apple-Notes-style layout: three <splitview> panes, each with its own
// <toolbarview>+<headerbar> (own header, not one shared window titlebar):
//   - sidebar (slot="sidebar", glass): folders (All Notes / Personal / Work
//     / Trash) as navigation-sidebar rows, counts in the label.
//   - list (slot="list"): search + a <sourcelist> of notes + a count caption.
//   - content (slot="content", declared LAST so GTK homes the menu bar's
//     primary button in ITS headerbar per GNOME convention): the floating
//     editing buttons (pin/delete/new-note, icon-only, end slot) plus the
//     title textinput, the body textarea, and the word-count/saved caption.
// A <menubar> (File > New Note, Note > Pin/Unpin + Delete) sits alongside the
// splitview as a Window sibling; its custom items call the EXACT SAME
// updater functions as the matching header buttons (createNote/togglePin/
// deleteSelected): one mutation path per action, two triggers each.
//
// Visual approach: native chrome, not hand-rolled facsimiles of it.
//   - The `<window>` is edge-to-edge (AdwApplicationWindow): the `<splitview>`
//     fills to the very top, GNOME-style.
//   - `<splitview>` is the real three-pane split (AdwOverlaySplitView nested
//     pair on GTK, NSSplitView three-way on Mac), not hand-rolled boxes.
//   - `cssClasses` reaches libadwaita's named classes (navigation-sidebar,
//     suggested-action, destructive-action, accent, dimmed, caption, view,
//     flat). libadwaita is initialized host-side (src/gtk/main.zig calls
//     adw.init()), so AdwStyleManager tracks the system color scheme — the
//     whole app follows light/dark automatically, with no per-widget color
//     values.
//   - `style` is kept only for theme-neutral geometry (padding, font size)
//     and layout (hexpand/vexpand/halign/valign — GTK widget properties
//     that drive the layout engine). No color literals anywhere: dark mode
//     is automatic on both platforms, and pin state surfaces as the
//     <sourcelist> row's leading star icon rather than a per-row border.

type FolderId = "personal" | "work";
type FolderView = "all" | FolderId | "trash";

interface Note {
  id: number;
  title: string;
  body: string;
  pinned: boolean;
  folderId: FolderId;
  deleted: boolean;
}

let nextId = 3;

const initialNotes: Note[] = [
  { id: 1, title: "Welcome to ND Notes", body: "This is your first note. Select it, edit it, or create a new one.", pinned: true, folderId: "personal", deleted: false },
  { id: 2, title: "Shopping list", body: "Milk\nEggs\nBread", pinned: false, folderId: "personal", deleted: false },
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

function folderScoped(all: Note[], view: FolderView): Note[] {
  switch (view) {
    case "trash":
      return all.filter((n) => n.deleted);
    case "personal":
      return all.filter((n) => !n.deleted && n.folderId === "personal");
    case "work":
      return all.filter((n) => !n.deleted && n.folderId === "work");
    case "all":
      return all.filter((n) => !n.deleted);
  }
}

const folders: { id: FolderView; label: string }[] = [
  { id: "all", label: "All Notes" },
  { id: "personal", label: "Personal" },
  { id: "work", label: "Work" },
  { id: "trash", label: "Trash" },
];

function App() {
  const [store, setStore] = createStore({ notes: initialNotes });
  const [selectedId, setSelectedId] = createSignal<number | null>(1);
  const [folder, setFolder] = createSignal<FolderView>("all");
  const [query, setQuery] = createSignal("");

  const scopedNotes = createMemo(() => folderScoped(store.notes, folder()));
  const filtered = createMemo(() => {
    const sorted = sortNotes(scopedNotes());
    const q = query().trim().toLowerCase();
    if (q === "") return sorted;
    return sorted.filter((n) => n.title.toLowerCase().includes(q) || n.body.toLowerCase().includes(q));
  });

  const count = (view: FolderView): number => folderScoped(store.notes, view).length;
  const selected = createMemo(() => store.notes.find((n) => n.id === selectedId()) ?? null);

  function updateSelected(patch: Partial<Note>): void {
    const id = selectedId();
    setStore((s) => {
      const note = s.notes.find((n) => n.id === id);
      if (note) Object.assign(note, patch);
    });
  }

  function togglePinSelected(): void {
    const note = selected();
    if (note == null) return;
    updateSelected({ pinned: !note.pinned });
  }

  function selectFolder(next: FolderView): void {
    setFolder(next);
    setQuery("");
    const scoped = sortNotes(folderScoped(store.notes, next));
    setSelectedId(scoped.length > 0 ? scoped[0]!.id : null);
  }

  function createNote(): void {
    // New notes land in the selected folder, or Personal when created from
    // All Notes / Trash (a folder view that isn't a real destination).
    const current = folder();
    const targetFolder: FolderId = current === "personal" || current === "work" ? current : "personal";
    const note: Note = { id: nextId++, title: "Untitled note", body: "", pinned: false, folderId: targetFolder, deleted: false };
    setStore((s) => {
      s.notes.push(note);
    });
    setSelectedId(note.id);
    setQuery("");
    if (current === "trash") setFolder(targetFolder);
  }

  function deleteSelected(): void {
    const note = selected();
    if (note == null) return;
    // Already in Trash: a permanent delete. Otherwise the note moves to Trash.
    const remaining = note.deleted
      ? store.notes.filter((n) => n.id !== note.id)
      : store.notes.map((n) => (n.id === note.id ? { ...n, deleted: true } : n));
    setStore((s) => {
      if (note.deleted) s.notes.splice(s.notes.findIndex((n) => n.id === note.id), 1);
      else s.notes.find((n) => n.id === note.id)!.deleted = true;
    });
    const nextScoped = sortNotes(folderScoped(remaining, folder()));
    setSelectedId(nextScoped.length > 0 ? nextScoped[0]!.id : null);
  }

  return (
    <window title="ND Notes" defaultWidth={1100} defaultHeight={700}>
      <menubar defaults>
        <menu label="File" testID="menu-file">
          <menuitem
            testID="menu-new-note"
            label="New Note"
            iconName="document-new"
            accelerator="primary+n"
            onSelect={createNote}
          />
        </menu>
        <menu label="Note" testID="menu-note">
          <menuitem
            testID="menu-toggle-pin"
            label={selected()?.pinned ? "Unpin" : "Pin"}
            accelerator="primary+p"
            enabled={selected() != null}
            onSelect={togglePinSelected}
          />
          <menuitem role="separator" testID="menu-note-sep" />
          <menuitem
            testID="menu-delete-note"
            label="Delete"
            iconName="edit-delete"
            accelerator="primary+backspace"
            enabled={selected() != null}
            onSelect={deleteSelected}
          />
        </menu>
      </menubar>

      <splitview sidebarWidth={0.2} listWidth={0.3} testID="split">
        <toolbarview slot="sidebar" testID="sidebar-toolbar">
          <headerbar testID="sidebar-header" title="ND Notes" />
          <box
            testID="sidebar-content"
            orientation="vertical"
            spacing={0}
            cssClasses={["navigation-sidebar"]}
            style={{ vexpand: true }}
          >
            {/* Row metrics (height, insets, 2px pitch, radius) and the
                selected/hover fills come from the framework, so the rows
                carry no geometry of their own. Two props still do real work:
                suggested-action is the selection signal both backends read
                (GTK's row fill, AppKit's .sourceList selection), and
                labelAlign, because GTK4 CSS has no text-align and a
                GtkButton label centres by default. */}
            <For each={folders}>
              {(f) => (
                <button
                  testID={`folder-row-${f.id}`}
                  label={`${f.label}  ${count(f.id)}`}
                  labelAlign="start"
                  onClick={() => selectFolder(f.id)}
                  cssClasses={folder() === f.id ? ["suggested-action"] : []}
                  style={{ halign: "fill" }}
                />
              )}
            </For>
          </box>
        </toolbarview>

        <toolbarview slot="list" testID="list-toolbar">
          <headerbar testID="list-header" title={folders.find((f) => f.id === folder())?.label} />
          <box testID="list-content" orientation="vertical" spacing={8} style={{ vexpand: true, padding: { top: 8, bottom: 8, left: 10, right: 10 } }}>
            <searchinput
              testID="search-input"
              text={query()}
              placeholder="Search notes"
              onChanged={(e) => setQuery(e.text)}
            />
            {/* SourceList is its own scroll container on both backends (GTK:
                GtkListBox in a ScrolledWindow; Mac: NSTableView .sourceList
                in an NSScrollView), so it needs no ScrollView wrapper.
                Selection is controlled (selectedIndex/onSelectionChanged);
                pin state
                surfaces as a leading star icon (source-list rows carry no
                per-row border), and the trailing badge is the note's live
                word count. */}
            <sourcelist
              testID="note-list"
              style={{ vexpand: true }}
              items={filtered().map((n) => ({
                title: n.title || "Untitled note",
                iconName: n.pinned ? "starred-symbolic" : undefined,
                badge: wordCount(n.body) > 0 ? String(wordCount(n.body)) : undefined,
              }))}
              selectedIndex={filtered().findIndex((n) => n.id === selectedId())}
              onSelectionChanged={(e) => {
                const n = filtered()[e.index];
                if (n) setSelectedId(n.id);
              }}
            />
            <label
              testID="note-count-label"
              text={`${filtered().length} of ${scopedNotes().length} note${scopedNotes().length === 1 ? "" : "s"}`}
              cssClasses={["dimmed", "caption"]}
            />
          </box>
        </toolbarview>

        <toolbarview slot="content" testID="content-toolbar">
          <headerbar testID="content-header" title="Editor">
            <button
              slot="end"
              testID="pin-button"
              iconName="view-pin"
              label=""
              tooltip="Pin"
              onClick={togglePinSelected}
              cssClasses={selected()?.pinned ? ["flat", "accent"] : ["flat"]}
            />
            <button slot="end" testID="delete-note-button" iconName="edit-delete" label="" tooltip="Delete" onClick={deleteSelected} cssClasses={["flat"]} />
            <button slot="end" testID="new-note-button" iconName="document-new" label="" tooltip="New Note" onClick={createNote} cssClasses={["flat"]} />
          </headerbar>
          {/* vexpand: the content pane claims all the height the header doesn't
              use. hexpand is not needed: a Box fills its parent's cross axis
              by default on both backends. */}
          <box testID="content-body" orientation="vertical" spacing={12} cssClasses={["view"]} style={{ vexpand: true, padding: 20 }}>
            <Show
              when={selected()}
              fallback={<label testID="empty-state-label" text="No note selected. Create one to get started." cssClasses={["dimmed"]} />}
            >
              {(note) => (
                <box orientation="vertical" spacing={12} style={{ vexpand: true }}>
                  <textinput
                    testID="title-input"
                    text={note().title}
                    placeholder="Title"
                    onChanged={(e) => updateSelected({ title: e.text })}
                    style={{ font: { fontSize: 20, fontWeight: "bold" }, padding: 4 }}
                  />
                  <separator orientation="horizontal" />
                  {/* TextArea has its own minContentHeight floor, so it needs no
                      ScrollView wrapper; vexpand lets it fill the pane. */}
                  <textarea
                    testID="editor-textarea"
                    minContentHeight={320}
                    text={note().body}
                    onChanged={(e) => updateSelected({ body: e.text })}
                    style={{ font: { fontSize: 14 }, padding: 4, vexpand: true }}
                  />
                  <label
                    testID="status-label"
                    text={`${wordCount(note().body)} word${wordCount(note().body) === 1 ? "" : "s"} · Saved`}
                    cssClasses={["dimmed", "caption"]}
                  />
                </box>
              )}
            </Show>
          </box>
        </toolbarview>
      </splitview>
    </window>
  );
}

await render(() => <App />);
