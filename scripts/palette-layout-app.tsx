// Fixture for scripts/palette-layout-drive.ts: a command bar the way a browser
// fills one. Long titles and URLs, a favicon, symbol icons, a row with no icon,
// right-aligned hints, a title that opens on a colour emoji, and inline
// completion on the first row. The window width comes from PALETTE_WIDTH so
// the drive can run it normal and narrow.
import { render } from "@nativedesktop/react";
import { createSignal } from "solid-js";

const FAVICON =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAMUlEQVR4nGNgoDZ4FqXxHx+mSDNeQ4jVjNUQUjVjGDJqwLAwgOKERJWkTKwheDWTAwB+c51E2IyQHgAAAABJRU5ErkJggg==";

const SITES = ["github.com", "gitlab.com", "news.ycombinator.com"];

interface Item {
  id: string;
  title: string;
  subtitle?: string;
  iconName?: string;
  iconData?: string;
  hint?: string;
  completion?: string;
}

function rank(query: string): Item[] {
  const q = query.trim().toLowerCase();
  const rows: Item[] = [];
  const site = q ? SITES.find((s) => s.startsWith(q)) : undefined;
  if (site) {
    rows.push({ id: `open:${site}`, title: site, subtitle: `https://${site}/`, iconData: FAVICON, hint: "Open", completion: site });
  }
  if (q) rows.push({ id: "search", title: `Search Google for ${query}`, iconName: "system-search-symbolic", hint: "Search" });
  rows.push(
    {
      id: "tab:long",
      title: "Quarterly engineering review and roadmap for the platform team, second half of the fiscal year",
      subtitle: "https://docs.example.com/engineering/reviews/2026/q3/roadmap?team=platform&view=full#milestones",
      iconData: FAVICON,
      hint: "Switch to Tab",
    },
    { id: "tab:short", title: "Inbox", subtitle: "https://mail.example.com/", iconName: "web-browser-symbolic", hint: "Switch to Tab" },
    { id: "tab:emoji", title: "⚡ Zig Programming Language", subtitle: "https://ziglang.org/", iconData: FAVICON, hint: "Switch to Tab" },
    { id: "hist:plain", title: "A row with no icon and no URL", hint: "Open" },
    { id: "cmd:new-tab", title: "New Tab", iconName: "tab-new-symbolic", hint: "Ctrl+T" },
    { id: "cmd:settings", title: "Settings", iconName: "preferences-system-symbolic", hint: "Ctrl+Comma" },
  );
  return rows;
}

function App() {
  const [open, setOpen] = createSignal(true);
  const [seed, setSeed] = createSignal("");
  const [query, setQuery] = createSignal("");
  const [last, setLast] = createSignal("(none)");
  return (
    <window title="Palette layout" defaultWidth={Number(process.env.PALETTE_WIDTH ?? 1280)} defaultHeight={800}>
      <box orientation="vertical" spacing={8}>
        <label testID="query" text={`Query: ${query()}`} />
        <label testID="last" text={`Last: ${last()}`} />
        <button
          testID="reopen"
          label="Open"
          onClick={() => {
            setSeed("https://example.com/current");
            setQuery("https://example.com/current");
            setOpen(true);
          }}
        />
        <button testID="reopen-empty" label="Open empty" onClick={() => setOpen(true)} />
        <commandpalette
          testID="palette"
          open={open()}
          placeholder="Search or enter address"
          query={seed()}
          items={rank(query())}
          onQueryChanged={(e) => setQuery(e.text)}
          onActivate={(e) => {
            setLast(`activate ${e.text}`);
            setOpen(false);
            setSeed("");
            setQuery("");
          }}
          onSubmit={(e) => {
            setLast(`submit ${e.text}`);
            setOpen(false);
            setSeed("");
            setQuery("");
          }}
          onCancel={() => {
            setLast("cancel");
            setOpen(false);
            setSeed("");
            setQuery("");
          }}
        />
      </box>
    </window>
  );
}

await render(() => <App />);
