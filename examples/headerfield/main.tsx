import { createPortal, render, sendCommand, useRef, useState } from "@nativedesktop/react";
import type { NdNodeRef } from "@nativedesktop/react";

// The browser-toolbar shape: nav buttons at the start, an address field that
// takes the whole run between the packs, actions at the end, and a site-info
// padlock INSIDE the field opening a popover anchored to the icon itself.
// Driven by scripts/headerfield-drive.ts on both backends.

function App(): React.ReactNode {
  const field = useRef<NdNodeRef<"searchinput">>(null);
  const [url, setUrl] = useState("example.com");
  const [icons, setIcons] = useState(0);
  const [info, setInfo] = useState(false);

  return (
    <window title="ND Header Field" defaultWidth={1100} defaultHeight={420}>
      <toolbarview>
        <headerbar slot="top" testID="header">
          <button slot="start" testID="nav-back" iconName="go-previous-symbolic" />
          <button slot="start" testID="nav-forward" iconName="go-next-symbolic" />
          <button slot="start" testID="nav-reload" iconName="view-refresh-symbolic" />
          <button
            slot="start"
            testID="weight-styled"
            label="Bookmarks"
            style={{ font: { fontWeight: "normal" } }}
          />
          <button slot="start" testID="weight-plain" label="Bookmarks" />
          <searchinput
            ref={field}
            testID="address"
            text={url}
            placeholder="Search or enter address"
            leadingIconName="channel-secure-symbolic"
            leadingIconTooltip="Connection is secure"
            leadingIconLabel="Site information"
            onChanged={(e) => setUrl(e.text)}
            onLeadingIconClicked={() => {
              setIcons((n) => n + 1);
              setInfo(true);
            }}
          />
          <button slot="end" testID="end-menu" iconName="open-menu-symbolic" />
          <button slot="end" testID="end-add" iconName="list-add-symbolic" />
        </headerbar>
        <box orientation="vertical" spacing={8} style={{ padding: 16 }}>
          <label testID="url-label" text={url} />
          <label testID="icon-count" text={`icon clicks: ${icons}`} />
          {/* The icon is not a widget of its own on either backend, so this is
              how an app (and the gate on GTK, where GTK4 refuses synthesized
              input) reaches it without a pointer. */}
          <button
            testID="fire-icon"
            label="Open site info"
            onClick={() => {
              if (field.current) sendCommand(field.current, "activateLeadingIcon");
            }}
          />
        </box>
      </toolbarview>
      {createPortal(
        <popover
          testID="site-info"
          anchorRef={field}
          anchorSlot="leadingIcon"
          open={info}
          position="bottom"
          onClosed={() => setInfo(false)}
        >
          <box testID="site-info-body" orientation="vertical" spacing={8}>
            <label testID="site-info-label" text="Connection is secure" />
          </box>
        </popover>,
      )}
    </window>
  );
}

render(<App />);
