import { render, useMemo, useState } from "@nativedesktop/react";

// Controlled command palette as a remote-style directory picker: the app owns
// `query` and `items`, recomputes results per keystroke (a real client would
// re-fetch over an RPC here), and the widget only renders + reports. onActivate
// drills into the highlighted folder; onSubmit accepts the typed path as-is
// (even when it matches no listed row); onCancel closes.

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
  }, [cwd, query]);

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
    <window title="Command Palette" defaultWidth={520} defaultHeight={280}>
      <box orientation="vertical" spacing={8}>
        <label testID="cwd-label" text={`Folder: ${cwd}`} />
        <label testID="picked-label" text={`Picked: ${picked}`} />
        <button testID="open-button" label="Open picker (Cmd-K)" onClick={openPalette} />
        {/* Controlled + always mounted: `open` toggles presentation, `query`
            and `items` are owned here and fed back every keystroke. */}
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
