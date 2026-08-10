import { render, useEffect, useMemo, useState } from "@nativedesktop/react";

// Controlled command palette as a remote-style directory picker: the app owns
// `query` and `items`, recomputes results per keystroke (a real client would
// re-fetch over an RPC here), and the widget only renders + reports. onActivate
// drills into the highlighted folder; onSubmit accepts the typed path as-is
// (even when it matches no listed row); onCancel closes.
//
// The palette is mounted as a child of a window that also holds other content,
// and a background tick re-renders the tree ~1.4x/sec so the controlled `items`
// array is rebuilt on every render (a live client re-fetches on a poll). The
// palette has to survive that churn: highlight, keyboard focus and row
// activation stay live across rebuilds, and it presents over the app's active
// window regardless of where its handle sits in the tree.

const FS: Record<string, string[]> = {
  "/": ["Users", "Applications", "System", "tmp"],
  "/Users": ["kyan", "shared"],
  "/Users/kyan": ["Developer", "Documents", "Downloads", "notes.md"],
  "/Users/kyan/Developer": ["NativeDesktop", "CanaryOrchestrator", "scratch.txt"],
  "/Users/kyan/Developer/NativeDesktop": ["src", "swift", "schema", "README.md"],
};

function childrenOf(dir: string): string[] {
  return FS[dir] ?? [];
}
function isDir(path: string): boolean {
  return FS[path] !== undefined;
}
function join(dir: string, name: string): string {
  return dir === "/" ? `/${name}` : `${dir}/${name}`;
}

interface Row {
  id: string;
  title: string;
  subtitle: string;
  iconName: string;
}

function App(): React.ReactNode {
  const [open, setOpen] = useState(false);
  const [cwd, setCwd] = useState("/Users/kyan");
  const [query, setQuery] = useState("");
  const [picked, setPicked] = useState("(nothing yet)");
  const [tick, setTick] = useState(0);

  // Live-app churn: rebuild the controlled tree on a poll while the palette is
  // open. `tick` feeds the items memo so a fresh array reaches the widget every
  // render, exercising the rebuild path a real re-fetch would hit.
  useEffect(() => {
    const id = setInterval(() => setTick((t) => t + 1), 700);
    return () => clearInterval(id);
  }, []);

  const items = useMemo<Row[]>(() => {
    const q = query.toLowerCase();
    return childrenOf(cwd)
      .filter((name) => name.toLowerCase().includes(q))
      .map((name) => {
        const path = join(cwd, name);
        const dir = isDir(path);
        return {
          id: path,
          title: name,
          subtitle: path,
          iconName: dir ? "folder" : "text-x-generic",
        };
      });
  }, [cwd, query, tick]);

  const openPalette = (): void => {
    setQuery("");
    setOpen(true);
  };

  const onActivate = (e: { text: string }): void => {
    const path = e.text;
    if (isDir(path)) {
      setCwd(path);
      setQuery("");
    } else {
      setPicked(path);
      setOpen(false);
    }
  };

  const onSubmit = (e: { text: string }): void => {
    const raw = e.text.trim();
    const path = raw.length === 0 ? cwd : raw.startsWith("/") ? raw : join(cwd, raw);
    setPicked(path);
    setOpen(false);
  };

  return (
    <window title="Command Palette" defaultWidth={760} defaultHeight={560}>
      <box orientation="vertical" spacing={8}>
        <label testID="tick-label" text={`Live tick: ${tick}`} />
        <label testID="cwd-label" text={`Folder: ${cwd}`} />
        <label testID="query-label" text={`Query: ${query}`} />
        <label testID="picked-label" text={`Picked: ${picked}`} />
        <button testID="open-button" label="Open picker (Cmd-K)" onClick={openPalette} />
        {/* Controlled + always mounted, nested beside other content: `open`
            toggles presentation, `query` and `items` are owned here and fed back
            every keystroke and on every background tick. */}
        <commandpalette
          testID="palette"
          open={open}
          placeholder="Search this folder, or type a path and press Return"
          query={query}
          items={items}
          onQueryChanged={(e) => setQuery(e.text)}
          onActivate={onActivate}
          onSubmit={onSubmit}
          onCancel={() => setOpen(false)}
        />
      </box>
    </window>
  );
}

await render(<App />);
