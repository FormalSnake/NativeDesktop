import { render, useState } from "@nativedesktop/react";

// Inspector example — the acceptance surface for the HIG design-gap batch:
//   - a `slot="inspector"` splitview pane (NSSplitViewItem inspector on the
//     Mac, an end-positioned AdwOverlaySplitView sidebar on GTK),
//   - a `prominent` toolbar button (accent-tinted item / suggested-action),
//   - a `badge` count on a toolbar button (NSItemBadge / capsule suffix),
//   - a `pill`-classed count label (native capsule badge on both backends).
// Driven by scripts/ndshot for the visual pass; interactions are plain state.

function App(): React.ReactNode {
  const [inbox, setInbox] = useState(3);
  const [saved, setSaved] = useState(0);

  return (
    <window title="ND Inspector" defaultWidth={1000} defaultHeight={620}>
      <splitview testID="split">
        <toolbarview slot="sidebar" testID="sidebar-pane">
          <headerbar testID="sidebar-header" title="Mailboxes" />
          <box orientation="vertical" testID="sidebar-content" style={{ padding: 8 }}>
            <box orientation="horizontal" cssClasses={["activatable"]} style={{ padding: { top: 6, bottom: 6, left: 12, right: 8 } }}>
              <label text="Inbox" style={{ hexpand: true }} />
              <label testID="inbox-pill" text={String(inbox)} cssClasses={["pill", "numeric", "caption"]} />
            </box>
            <box orientation="horizontal" cssClasses={["activatable"]} style={{ padding: { top: 6, bottom: 6, left: 12, right: 8 } }}>
              <label text="Archive" style={{ hexpand: true }} />
            </box>
          </box>
        </toolbarview>
        <toolbarview slot="content" testID="content-pane">
          <headerbar testID="content-header" title="Message">
            <button
              slot="end"
              testID="save-button"
              label=""
              iconName="document-save"
              prominent
              onClick={() => setSaved((n) => n + 1)}
            />
            <button
              slot="end"
              testID="inbox-button"
              label=""
              iconName="mail-unread"
              badge={String(inbox)}
              onClick={() => setInbox((n) => n + 1)}
            />
          </headerbar>
          {/* The pane root is the textarea itself (scroll-shaped), so the
              message body extends edge-to-edge under the glass toolbar and
              gets the scroll edge effect while scrolled. */}
          <textarea
            testID="body-editor"
            text={Array.from({ length: 60 }, (_, i) => `Line ${i + 1}: the quick brown fox jumps over the lazy dog.`).join("\n")}
            minContentHeight={240}
            style={{ hexpand: true, vexpand: true }}
          />
        </toolbarview>
        <toolbarview slot="inspector" testID="inspector-pane">
          <headerbar testID="inspector-header" title="Details" />
          <box orientation="vertical" testID="inspector-content" style={{ padding: 12 }}>
            <label text="Details" cssClasses={["heading"]} />
            <label testID="saved-label" text={`Saved ${saved} times, ${inbox} unread`} cssClasses={["caption"]} />
            <label text="From: nd@example.com" cssClasses={["caption"]} />
            <label text="Attachments: 2" cssClasses={["caption"]} />
            <button
              testID="mark-read"
              label="Mark as read"
              size="small"
              onClick={() => setInbox((n) => Math.max(0, n - 1))}
            />
          </box>
        </toolbarview>
      </splitview>
    </window>
  );
}

await render(<App />);
