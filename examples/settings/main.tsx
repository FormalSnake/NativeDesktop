import { render, useState, Platform } from "@nativedesktop/react";

// ND Settings — a two-pane preferences window. The SAME tree renders grouped
// forms as native Adwaita "boxed-list" cards on GTK and as clean, chrome-free
// forms on the Mac.
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
//     vexpand, halign), with no color literals; dark mode is automatic.
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
  listDensityIndex: 0,
  devMode: false,
  autosaveInterval: 5,
};

function App(): React.ReactNode {
  const [category, setCategory] = useState<Category>("general");
  // Minimal visited-category history so the content header's native back/
  // forward chevrons have something to drive. `historyIndex` points at the
  // current entry; picking a category truncates any forward entries (standard
  // browser-history semantics).
  const [history, setHistory] = useState<Category[]>(["general"]);
  const [historyIndex, setHistoryIndex] = useState(0);
  // Gives the theme radios a fresh group name each time the Appearance
  // category is re-entered. This routed around a GTK use-after-free:
  // remounting radios under the same group name joined a freed CheckButton.
  // The backend now evicts a group's anchor on destroy
  // (cbRadioGroupDestroyed, src/generated/widgets.zig), so reusing "theme"
  // would be safe; the epoch stays as a harmless guard.
  const [appearanceEpoch, setAppearanceEpoch] = useState(0);
  const [launchAtLogin, setLaunchAtLogin] = useState(defaults.launchAtLogin);
  const [showStatusIcon, setShowStatusIcon] = useState(defaults.showStatusIcon);
  const [folderIndex, setFolderIndex] = useState(defaults.folderIndex);
  const [theme, setTheme] = useState<Theme>(defaults.theme);
  const [textSize, setTextSize] = useState(defaults.textSize);
  const [listDensityIndex, setListDensityIndex] = useState(defaults.listDensityIndex);
  const [devMode, setDevMode] = useState(defaults.devMode);
  const [autosaveInterval, setAutosaveInterval] = useState(defaults.autosaveInterval);

  function resetAll(): void {
    setLaunchAtLogin(defaults.launchAtLogin);
    setShowStatusIcon(defaults.showStatusIcon);
    setFolderIndex(defaults.folderIndex);
    setTheme(defaults.theme);
    setTextSize(defaults.textSize);
    setListDensityIndex(defaults.listDensityIndex);
    setDevMode(defaults.devMode);
    setAutosaveInterval(defaults.autosaveInterval);
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
    const previous = history[i];
    if (!previous) return;
    setHistoryIndex(i);
    selectCategory(previous);
  }

  function goForward(): void {
    if (historyIndex >= history.length - 1) return;
    const i = historyIndex + 1;
    const next = history[i];
    if (!next) return;
    setHistoryIndex(i);
    selectCategory(next);
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
              // AppKit aligns the first card with the sidebar's leading item
              // (System Settings), so its top inset matches the sidebar's;
              // Adwaita wants even margins on every side. Same tree, native each.
              style={{ hexpand: true, padding: { top: Platform.select({ appkit: 6, gtk: 24 }), right: 24, bottom: 24, left: 24 } }}
            >
              {category === "general" && (
                <settingsgroup spacing={0} testID="general-card">
                  <box orientation="horizontal" style={{ padding: 12 }}>
                    <checkbox
                      testID="setting-launch"
                      label="Launch at login"
                      checked={launchAtLogin}
                      onToggled={(e) => setLaunchAtLogin(e.checked)}
                    />
                  </box>
                  <separator orientation="horizontal" />
                  <box orientation="horizontal" spacing={12} style={{ padding: 12 }}>
                    <label text="Show status icon" style={{ hexpand: true, valign: "center" }} />
                    <switch
                      testID="setting-status-icon"
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
                </settingsgroup>
              )}

              {category === "appearance" && (
                <box orientation="vertical" spacing={20} testID="appearance-card">
                  <settingsgroup spacing={0}>
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
                  </settingsgroup>
                  <settingsgroup spacing={8} style={{ padding: 12 }}>
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
                  </settingsgroup>
                  <settingsgroup spacing={8} style={{ padding: 12 }}>
                    <label text="List density" cssClasses={["heading"]} />
                    <segmentedcontrol
                      testID="setting-list-density"
                      options={["Comfortable", "Compact"]}
                      selectedIndex={listDensityIndex}
                      onSelectionChanged={(e) => setListDensityIndex(e.index)}
                    />
                    <label
                      testID="setting-list-density-caption"
                      text={listDensityIndex === 0 ? "More breathing room between rows." : "Tighter rows, more content per screen."}
                      cssClasses={["dimmed", "caption"]}
                    />
                  </settingsgroup>
                </box>
              )}

              {category === "advanced" && (
                <box orientation="vertical" spacing={20} testID="advanced-card">
                  <settingsgroup spacing={0}>
                    <box orientation="horizontal" style={{ padding: 12 }}>
                      <checkbox
                        testID="setting-devmode"
                        label="Developer mode"
                        checked={devMode}
                        onToggled={(e) => setDevMode(e.checked)}
                      />
                    </box>
                  </settingsgroup>
                  <settingsgroup spacing={8} style={{ padding: 12 }}>
                    <label text="Autosave interval" cssClasses={["heading"]} />
                    <box orientation="horizontal" spacing={8}>
                      <numberinput
                        testID="setting-autosave-interval"
                        value={autosaveInterval}
                        min={1}
                        max={60}
                        step={1}
                        digits={0}
                        onValueChanged={(e) => setAutosaveInterval(e.value)}
                      />
                      <label text="minutes" style={{ valign: "center" }} cssClasses={["dimmed"]} />
                    </box>
                  </settingsgroup>
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
