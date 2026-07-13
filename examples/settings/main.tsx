import { render, useState } from "@nativedesktop/react";

// ND Settings — a two-pane preferences window, proving the M13 chrome
// machinery renders grouped forms as native Adwaita "boxed-list" cards on
// GTK and clean, chrome-free forms on the Mac from the SAME tree.
//
// Visual approach, mirroring examples/notes/main.tsx's doctrine:
//   - `<splitview>` is the real sidebar/content split (AdwOverlaySplitView on
//     GTK, NSSplitView + vibrancy sidebar on Mac); each pane is a
//     `<toolbarview>` carrying its own `<headerbar>` (see examples/tasks/
//     main.tsx's header comment for why a `<toolbarview>` always lives
//     inside a `<splitview>` pane, never bare under `<window>`).
//   - There is deliberately NO `<menubar>` in this tree. That's the point of
//     this example: the Mac backend installs its standard default menu
//     chrome (App/File/Edit/View/Window/Help) unconditionally at window
//     creation, with zero platform code and zero declared menu — "defaults
//     are automatic" needs no proof beyond leaving them out. GTK has no such
//     automatic top-level chrome; with no `<menubar>` declared, no hamburger
//     button appears at all, which is legitimate, unsurprising GNOME
//     behavior for a plain preferences window.
//   - Category selection reuses Notes' note-row recipe: a
//     `cssClasses={["navigation-sidebar"]}` box of flat buttons,
//     `labelAlign="start"`, selection = `suggested-action`.
//   - Grouped forms use libadwaita's `boxed-list` card class (schema-listed
//     in `cssClasses`, docs/styling.md) on a vertical box holding one row
//     per setting, `<separator>`s between rows standing in for the list's
//     internal dividers. `style` stays geometry-only (padding, hexpand/
//     vexpand, halign) — no color literals; dark mode is automatic.
type Category = "general" | "appearance" | "advanced";
type Theme = "system" | "light" | "dark";

const categories: { id: Category; label: string }[] = [
  { id: "general", label: "General" },
  { id: "appearance", label: "Appearance" },
  { id: "advanced", label: "Advanced" },
];

const folderOptions = ["Documents", "Downloads", "Desktop"];

const defaults = {
  launchAtLogin: true,
  showStatusIcon: true,
  folderIndex: 0,
  theme: "system" as Theme,
  textSize: 14,
  devMode: false,
};

function App(): React.ReactNode {
  const [category, setCategory] = useState<Category>("general");
  // Minimal visited-category history so the content header's native back/
  // forward chevrons have something to drive. `historyIndex` points at the
  // current entry; picking a category truncates any forward entries (standard
  // browser-history semantics).
  const [history, setHistory] = useState<Category[]>(["general"]);
  const [historyIndex, setHistoryIndex] = useState(0);
  // Radio.group is a process-lifetime key on both backends (GTK's
  // radio_groups map, src/generated/widgets.zig, never releases its stored
  // "first" CheckButton pointer on unmount) — reusing the same group string
  // across a destroy+remount cycle dereferences a freed widget and crashes.
  // The Appearance card's theme radios ARE destroyed/remounted every time
  // this conditionally-rendered category is left and re-entered, so each
  // re-entry gets a fresh, never-before-seen group name instead of reusing
  // "theme" (a real backend bug, not an app concern to route around
  // otherwise — see the header comment).
  const [appearanceEpoch, setAppearanceEpoch] = useState(0);
  const [launchAtLogin, setLaunchAtLogin] = useState(defaults.launchAtLogin);
  const [showStatusIcon, setShowStatusIcon] = useState(defaults.showStatusIcon);
  const [folderIndex, setFolderIndex] = useState(defaults.folderIndex);
  const [theme, setTheme] = useState<Theme>(defaults.theme);
  const [textSize, setTextSize] = useState(defaults.textSize);
  const [devMode, setDevMode] = useState(defaults.devMode);

  function resetAll(): void {
    setLaunchAtLogin(defaults.launchAtLogin);
    setShowStatusIcon(defaults.showStatusIcon);
    setFolderIndex(defaults.folderIndex);
    setTheme(defaults.theme);
    setTextSize(defaults.textSize);
    setDevMode(defaults.devMode);
  }

  function selectCategory(next: Category): void {
    if (next === "appearance" && category !== "appearance") {
      setAppearanceEpoch((e) => e + 1);
    }
    setCategory(next);
  }

  // Sidebar pick: navigate AND record history.
  function pickCategory(next: Category): void {
    if (next === category) return;
    setHistory((h) => [...h.slice(0, historyIndex + 1), next]);
    setHistoryIndex((i) => i + 1);
    selectCategory(next);
  }

  function goBack(): void {
    if (historyIndex <= 0) return;
    const i = historyIndex - 1;
    setHistoryIndex(i);
    selectCategory(history[i]);
  }

  function goForward(): void {
    if (historyIndex >= history.length - 1) return;
    const i = historyIndex + 1;
    setHistoryIndex(i);
    selectCategory(history[i]);
  }

  const themeGroup = `theme-${appearanceEpoch}`;

  return (
    <window title="Settings" defaultWidth={720} defaultHeight={480}>
      <splitview sidebarWidth={0.32} testID="settings-split">
        <toolbarview slot="sidebar" testID="settings-sidebar-toolbar">
          <headerbar title="Settings" testID="settings-sidebar-header" />
          <box
            orientation="vertical"
            spacing={2}
            cssClasses={["navigation-sidebar"]}
            style={{ vexpand: true, padding: { top: 12, bottom: 12, left: 8, right: 8 } }}
          >
            {categories.map((c) => (
              <button
                key={c.id}
                testID={`category-${c.id}`}
                label={c.label}
                labelAlign="start"
                onClick={() => pickCategory(c.id)}
                cssClasses={category === c.id ? ["suggested-action"] : ["flat"]}
                style={{ padding: { top: 8, bottom: 8, left: 10, right: 10 }, halign: "fill" }}
              />
            ))}
          </box>
        </toolbarview>

        <toolbarview slot="content" testID="settings-content-toolbar">
          {/* The content header carries the native floating back/forward
              chevrons (System Settings' leading `< >`), driven by the visited-
              category history above. Each segment greys out at the ends of the
              history — the framework renders the NSSegmentedControl. The `title`
              is the current page, shown as a small title right of the chevrons;
              `title` is a create-only prop, so `key={category}` remounts the
              header on navigation to pick up the new title. */}
          <headerbar
            key={category}
            testID="settings-content-header"
            title={categories.find((c) => c.id === category)?.label}
            canGoBack={historyIndex > 0}
            canGoForward={historyIndex < history.length - 1}
            onBack={goBack}
            onForward={goForward}
          />
          <scrollview testID="settings-content-scroll" minContentHeight={380} style={{ vexpand: true }}>
            <box
              orientation="vertical"
              spacing={20}
              cssClasses={["view"]}
              style={{ hexpand: true, padding: 24 }}
            >
              {category === "general" && (
                <box orientation="vertical" spacing={0} cssClasses={["boxed-list"]} testID="general-card">
                  <box orientation="horizontal" style={{ padding: 12 }}>
                    <checkbox
                      testID="setting-launch"
                      label="Launch at login"
                      checked={launchAtLogin}
                      onToggled={(e) => setLaunchAtLogin(e.checked)}
                    />
                  </box>
                  <separator orientation="horizontal" />
                  <box orientation="horizontal" style={{ padding: 12 }}>
                    <checkbox
                      testID="setting-status-icon"
                      label="Show status icon"
                      checked={showStatusIcon}
                      onToggled={(e) => setShowStatusIcon(e.checked)}
                    />
                  </box>
                  <separator orientation="horizontal" />
                  <box orientation="horizontal" spacing={12} style={{ padding: 12 }}>
                    <label text="Default folder" style={{ hexpand: true, valign: "center" }} />
                    <select
                      testID="setting-folder"
                      options={folderOptions}
                      selectedIndex={folderIndex}
                      onSelectionChanged={(e) => setFolderIndex(e.index)}
                    />
                  </box>
                </box>
              )}

              {category === "appearance" && (
                <box orientation="vertical" spacing={20} testID="appearance-card">
                  <box orientation="vertical" spacing={0} cssClasses={["boxed-list"]}>
                    <box orientation="horizontal" style={{ padding: 12 }}>
                      <radio
                        testID="setting-theme-system"
                        group={themeGroup}
                        label="System"
                        checked={theme === "system"}
                        onToggled={(e) => {
                          if (e.checked) setTheme("system");
                        }}
                      />
                    </box>
                    <separator orientation="horizontal" />
                    <box orientation="horizontal" style={{ padding: 12 }}>
                      <radio
                        testID="setting-theme-light"
                        group={themeGroup}
                        label="Light"
                        checked={theme === "light"}
                        onToggled={(e) => {
                          if (e.checked) setTheme("light");
                        }}
                      />
                    </box>
                    <separator orientation="horizontal" />
                    <box orientation="horizontal" style={{ padding: 12 }}>
                      <radio
                        testID="setting-theme-dark"
                        group={themeGroup}
                        label="Dark"
                        checked={theme === "dark"}
                        onToggled={(e) => {
                          if (e.checked) setTheme("dark");
                        }}
                      />
                    </box>
                  </box>
                  <box
                    orientation="vertical"
                    spacing={8}
                    cssClasses={["boxed-list"]}
                    style={{ padding: 12 }}
                  >
                    <label text="Text size" cssClasses={["heading"]} />
                    <slider
                      testID="setting-textsize"
                      min={10}
                      max={24}
                      step={1}
                      value={textSize}
                      onValueChanged={(e) => setTextSize(e.value)}
                    />
                    <label
                      testID="setting-textsize-caption"
                      text={`${Math.round(textSize)}pt`}
                      cssClasses={["dimmed", "caption"]}
                    />
                  </box>
                </box>
              )}

              {category === "advanced" && (
                <box orientation="vertical" spacing={20} testID="advanced-card">
                  <box orientation="vertical" spacing={0} cssClasses={["boxed-list"]}>
                    <box orientation="horizontal" style={{ padding: 12 }}>
                      <checkbox
                        testID="setting-devmode"
                        label="Developer mode"
                        checked={devMode}
                        onToggled={(e) => setDevMode(e.checked)}
                      />
                    </box>
                  </box>
                  <button
                    testID="reset-button"
                    label="Reset All Settings"
                    onClick={resetAll}
                    cssClasses={["destructive-action", "pill"]}
                    style={{ halign: "start" }}
                  />
                </box>
              )}
            </box>
          </scrollview>
        </toolbarview>
      </splitview>
    </window>
  );
}

await render(<App />);
