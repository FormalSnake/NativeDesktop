import { render, sendCommand, useRef, useState, type NdNodeRef } from "@nativedesktop/react";

// A browser-shaped window for scripts/header-palette-drive.ts: a search field
// packed straight into the header bar (the address field), and a command
// palette opened from a menu accelerator while that field may hold the
// keyboard. Cmd+L puts the caret in the field with its contents selected.

function App(): React.ReactNode {
  const [open, setOpen] = useState(false);
  const [committed, setCommitted] = useState("");
  const [picked, setPicked] = useState("");
  const omnibox = useRef<NdNodeRef<"searchinput"> | null>(null);

  return (
    <window title="Header Palette" defaultWidth={1280} defaultHeight={720}>
      <menubar defaults>
        <menu label="Go" testID="menu-go">
          <menuitem
            testID="menu-address"
            label="Open Location"
            accelerator="primary+l"
            onSelect={() => {
              if (omnibox.current) sendCommand(omnibox.current, "focus", { select: true });
            }}
          />
          <menuitem testID="menu-palette" label="Command Palette" accelerator="primary+k" onSelect={() => setOpen(true)} />
        </menu>
      </menubar>
      <toastoverlay>
        <splitview sidebarWidth={0.24} testID="split">
          <toolbarview slot="sidebar">
            <headerbar title="NativeBrowser" />
            <box orientation="vertical" style={{ vexpand: true, padding: { top: 8, bottom: 8 } }}>
              <button label="New Tab" iconName="tab-new-symbolic" labelAlign="start" cssClasses={["flat"]} style={{ hexpand: true }} />
            </box>
          </toolbarview>
          <toolbarview slot="content">
            <headerbar title="" testID="chrome" canGoBack={false} canGoForward={false} onBack={() => {}} onForward={() => {}}>
              <button slot="start" testID="layout-toggle" iconName="sidebar-show-symbolic" tooltip="Use Compact Layout" cssClasses={["flat"]} />
              <button slot="start" testID="reload" iconName="view-refresh-symbolic" tooltip="Reload" cssClasses={["flat"]} />
              <button slot="start" testID="security" iconName="web-browser-symbolic" tooltip="Site Information" cssClasses={["flat"]} />
              <searchinput
                slot="start"
                testID="omnibox"
                ref={(node) => {
                  omnibox.current = node as NdNodeRef<"searchinput"> | null;
                }}
                placeholder="Search or enter address"
                style={{ hexpand: true, minWidth: 240 }}
                onActivate={(e) => {
                  // A browser commits the address and drops its palette.
                  setCommitted(e.text);
                  setOpen(false);
                }}
              />
              <button slot="end" testID="extensions" iconName="application-x-addon-symbolic" tooltip="Extensions" cssClasses={["flat"]} />
              <button slot="end" testID="downloads" iconName="folder-download-symbolic" tooltip="Downloads" cssClasses={["flat"]} />
            </headerbar>
            <box orientation="vertical" spacing={8} style={{ vexpand: true, padding: { top: 12, left: 12, right: 12, bottom: 12 } }}>
              <label testID="committed-label" text={`Committed: ${committed}`} />
              <label testID="picked-label" text={`Picked: ${picked}`} />
              <commandpalette
                testID="palette"
                open={open}
                placeholder="Type a command"
                query=""
                items={[{ id: "reload", title: "Reload", subtitle: "", iconName: "view-refresh" }]}
                onActivate={(e) => {
                  setPicked(e.text);
                  setOpen(false);
                }}
                onSubmit={() => setOpen(false)}
                onCancel={() => setOpen(false)}
              />
            </box>
          </toolbarview>
        </splitview>
      </toastoverlay>
    </window>
  );
}

await render(<App />);
