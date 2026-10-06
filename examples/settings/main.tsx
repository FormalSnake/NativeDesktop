import { render } from "@nativedesktop/solid";
import { Match, Show, Switch, createMemo, createSignal, createStore } from "solid-js";

// ND Settings — a two-pane preferences window built from the framework's
// boxed-list widgets. The SAME tree renders real AdwPreferencesGroup /
// AdwActionRow / AdwSwitchRow chrome on GTK and SwiftUI grouped Form rows on
// the Mac — no hand-rolled box+separator rows, no per-app padding numbers.
//
// Structure, mirroring examples/notes/main.tsx's doctrine:
//   - `<splitview>` is the real sidebar/content split; each pane is a
//     `<toolbarview>` carrying its own `<headerbar>`. `breakpoint` collapses
//     the sidebar automatically on narrow windows (AdwBreakpoint on GTK).
//   - The sidebar is a `<sourcelist>` — the framework's flat navigation
//     list, not a box of styled buttons.
//   - Each page is a `<clamp>` (AdwClamp width ceiling) holding
//     `<settingsgroup title description>` cards of `<row>`/`<switchrow>`s.
//     Controls that need a custom widget (select/slider/numberinput) sit in
//     a row's suffix slot.
//   - The content header's `subtitle` shows the current page's blurb —
//     AdwWindowTitle's second line on GTK, NSWindow.subtitle on the Mac.
type Category = "general" | "appearance" | "advanced";
type Theme = "system" | "light" | "dark";

const categories: { id: Category; label: string; blurb: string }[] = [
  { id: "general", label: "General", blurb: "Startup and files" },
  { id: "appearance", label: "Appearance", blurb: "Theme and text" },
  { id: "advanced", label: "Advanced", blurb: "Here be dragons" },
];

const folderOptions = ["Documents", "Downloads", "Desktop"];
const themeOptions: Theme[] = ["system", "light", "dark"];

// A 16x16 solid magenta PNG. `iconData` takes raw image bytes where no theme
// icon exists (a favicon, a per-workspace image), and wins over `iconName`
// when a row sets both. Loud on purpose so a capture shows the bytes landed.
const FOLDER_ICON_DATA =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAGklEQVR42mP4z/D/PyWYYdSAUQNGDRguBgAAx4/9H5ua3FcAAAAASUVORK5CYII=";

const defaults = {
  launchAtLogin: true,
  showStatusIcon: true,
  folderIndex: 0,
  themeIndex: 0,
  textSize: 14,
  devMode: false,
  autosaveInterval: 5,
};

function App() {
  const [category, setCategory] = createSignal<Category>("general");
  // Minimal visited-category history so the content header's native back/
  // forward chevrons have something to drive (standard browser semantics).
  const [history, setHistory] = createSignal<Category[]>(["general"]);
  const [historyIndex, setHistoryIndex] = createSignal(0);
  const [settings, setSettings] = createStore({ ...defaults });
  const [lastUpdateCheck, setLastUpdateCheck] = createSignal<string | null>(null);
  // Last interaction, rendered in a caption at the page bottom — the
  // automation drive asserts state transitions through it (waitForText).
  const [status, setStatus] = createSignal("ready");

  function resetAll(): void {
    setSettings((s) => {
      Object.assign(s, defaults);
    });
    setStatus("settings reset");
  }

  function pickCategory(next: Category): void {
    if (next === category()) return;
    setHistory((h) => [...h.slice(0, historyIndex() + 1), next]);
    setHistoryIndex((i) => i + 1);
    setCategory(next);
  }

  function goBack(): void {
    const previous = history()[historyIndex() - 1];
    if (!previous) return;
    setHistoryIndex((i) => i - 1);
    setCategory(previous);
  }

  function goForward(): void {
    const next = history()[historyIndex() + 1];
    if (!next) return;
    setHistoryIndex((i) => i + 1);
    setCategory(next);
  }

  const current = createMemo(() => categories.find((c) => c.id === category()) ?? categories[0]!);

  return (
    <window title="Settings" defaultWidth={720} defaultHeight={480}>
      <splitview sidebarWidth={0.32} breakpoint={480} testID="settings-split">
        <toolbarview slot="sidebar" testID="settings-sidebar-toolbar">
          <headerbar title="Settings" testID="settings-sidebar-header" />
          <sourcelist
            testID="settings-categories"
            items={categories.map((c) => ({ title: c.label }))}
            selectedIndex={categories.findIndex((c) => c.id === category())}
            onSelectionChanged={(e) => {
              const picked = categories[e.index];
              if (picked) pickCategory(picked.id);
            }}
            style={{ vexpand: true }}
          />
        </toolbarview>

        <toolbarview slot="content" testID="settings-content-toolbar">
          {/* Native floating back/forward chevrons (System Settings' leading
              `< >`), driven by the visited-category history. `title` is
              create-only, so the keyed Show remounts the header per page;
              `subtitle` (createAndUpdate) carries the page blurb. */}
          <Show when={current()} keyed>
            {(page) => (
              <headerbar
                testID="settings-content-header"
                title={page.label}
                subtitle={page.blurb}
                canGoBack={historyIndex() > 0}
                canGoForward={historyIndex() < history().length - 1}
                onBack={goBack}
                onForward={goForward}
              />
            )}
          </Show>
          <scrollview testID="settings-content-scroll" minContentHeight={380} style={{ vexpand: true }}>
            <clamp maximumSize={560} testID="settings-clamp">
              <box orientation="vertical" spacing={18} style={{ hexpand: true, padding: { top: 18, bottom: 18, left: 12, right: 12 } }}>
                <Switch>
                  <Match when={category() === "general"}>
                    <settingsgroup title="General" testID="general-card">
                      <switchrow
                        testID="setting-launch"
                        title="Launch at login"
                        checked={settings.launchAtLogin}
                        onToggled={(e) => {
                          setSettings((s) => {
                            s.launchAtLogin = e.checked;
                          });
                          setStatus(`launch ${e.checked}`);
                        }}
                      />
                      <switchrow
                        testID="setting-status-icon"
                        title="Show status icon"
                        subtitle="Menu bar and tray presence"
                        checked={settings.showStatusIcon}
                        onToggled={(e) =>
                          setSettings((s) => {
                            s.showStatusIcon = e.checked;
                          })
                        }
                      />
                      <row title="Default folder" subtitle="Where new documents land" iconData={FOLDER_ICON_DATA}
                        testID="setting-folder-row">
                        <select
                          testID="setting-folder"
                          options={folderOptions}
                          selectedIndex={settings.folderIndex}
                          onSelectionChanged={(e) =>
                            setSettings((s) => {
                              s.folderIndex = e.index;
                            })
                          }
                        />
                      </row>
                    </settingsgroup>
                  </Match>

                  <Match when={category() === "appearance"}>
                    <settingsgroup title="Appearance" description="Theme changes apply immediately." testID="appearance-card">
                      <row title="Theme" testID="setting-theme-row">
                        <select
                          testID="setting-theme"
                          options={themeOptions.map((t) => t[0]!.toUpperCase() + t.slice(1))}
                          selectedIndex={settings.themeIndex}
                          onSelectionChanged={(e) =>
                            setSettings((s) => {
                              s.themeIndex = e.index;
                            })
                          }
                        />
                      </row>
                      <row title="Text size" subtitle={`${Math.round(settings.textSize)}pt`} testID="setting-textsize-row">
                        <slider
                          testID="setting-textsize"
                          min={10}
                          max={24}
                          step={1}
                          value={settings.textSize}
                          onValueChanged={(e) =>
                            setSettings((s) => {
                              s.textSize = e.value;
                            })
                          }
                          // hexpand propagates up into the row's suffix area, so
                          // the scale gets a usable track (GNOME Settings' slider
                          // rows do the same).
                          style={{ hexpand: true }}
                        />
                      </row>
                    </settingsgroup>
                  </Match>

                  <Match when={category() === "advanced"}>
                    <box orientation="vertical" spacing={18} testID="advanced-card">
                      <settingsgroup title="Advanced" description="These options can break things.">
                        <switchrow
                          testID="setting-devmode"
                          title="Developer mode"
                          subtitle="Verbose logging and unstable features"
                          checked={settings.devMode}
                          onToggled={(e) => {
                            setSettings((s) => {
                              s.devMode = e.checked;
                            });
                            setStatus(`devmode ${e.checked}`);
                          }}
                        />
                        <row title="Autosave interval" subtitle="Minutes between saves" testID="setting-autosave-row">
                          <numberinput
                            testID="setting-autosave-interval"
                            value={settings.autosaveInterval}
                            min={1}
                            max={60}
                            step={1}
                            digits={0}
                            onValueChanged={(e) =>
                              setSettings((s) => {
                                s.autosaveInterval = e.value;
                              })
                            }
                          />
                        </row>
                        <row
                          title="Check for updates"
                          subtitle={lastUpdateCheck() ? `Last checked ${lastUpdateCheck()}` : "Never checked"}
                          activatable
                          testID="check-updates-row"
                          onActivate={() => {
                            setLastUpdateCheck("just now");
                            setStatus("updates checked");
                          }}
                        />
                      </settingsgroup>
                      <button
                        testID="reset-button"
                        label="Reset All Settings"
                        onClick={resetAll}
                        cssClasses={["destructive-action", "pill"]}
                        style={{ halign: "start" }}
                      />
                    </box>
                  </Match>
                </Switch>

                <label testID="settings-status" text={status()} cssClasses={["caption", "dimmed"]} />
              </box>
            </clamp>
          </scrollview>
        </toolbarview>
      </splitview>
    </window>
  );
}

await render(() => <App />);
