---
title: "Native Tabs"
description: "Group <window> roots into real system tabs — NSWindow tab groups on macOS, AdwTabView with the tab overview on GNOME — from one unchanged app tree."
---

Give several `<window>` roots the same `tabGroup` and they render as one tabbed window, using each
platform's real tab system: on macOS every tab is a genuine `NSWindow` joined into a native tab
group (the Safari/Finder tab bar, drag a tab out to its own window, drag it back in, Show All
Tabs), and on GNOME the group renders as an `AdwTabView` with an autohiding `AdwTabBar` under your
header bar plus the `AdwTabButton` tab-overview toggle — the Ghostty setup. All tab chrome comes
from the framework; your app only owns the list of open tabs.

`examples/browser/main.tsx` and `examples/terminal/main.tsx` are the reference apps for this page.

## Tabs are windows

The app model is [multi-window](/native-platform/multi-window/) plus one prop. Each tab is a full
`<window>` root; `tabGroup` names the group it belongs to:

```tsx
import { render, useRef, useState } from "@nativedesktop/react";

function TerminalTab({ onNewTab, onClose }: { onNewTab: () => void; onClose: () => void }) {
  return (
    <window title="Terminal" tabGroup="terminal" onNewTabRequested={onNewTab} onClosed={onClose}>
      {/* any window content — its own headerbar, its own state */}
    </window>
  );
}

function App(): React.ReactNode {
  const [tabs, setTabs] = useState<number[]>([0]);
  const nextId = useRef(1);
  return (
    <>
      {tabs.map((id) => (
        <TerminalTab
          key={id}
          onNewTab={() => setTabs((open) => [...open, nextId.current++])}
          onClose={() => setTabs((open) => open.filter((t) => t !== id))}
        />
      ))}
    </>
  );
}

await render(<App />);
```

Because a tab is a window, everything window-scoped keeps working per tab unchanged: per-window
[dialogs](/components/dialogs/), toolbars, automation targeting, and the `core:window.create` ACL
gate (each new tab is a `create Window` op scoped to its own id).

## Native events

The framework owns the tab chrome, so tab lifecycle arrives as events on the `<window>` node:

- **`onNewTabRequested`** — the user asked for a tab natively: the tab bar's `+` button (macOS
  shows it because the framework implements `newWindowForTab` in the responder chain; GTK gets a
  framework-injected `+` on the `AdwTabBar`). Respond by rendering one more `<window tabGroup>`.
  The payload carries `{ tabGroup }`.
- **`onClosed`** — the user closed the tab (or the whole window). Respond by unmounting that
  `<window>`; the framework holds the native close pending until your unmount confirms it, riding
  AdwTabView's asynchronous `close-page` protocol on GTK and `windowShouldClose` on macOS. Plain
  ungrouped windows fire `onClosed` too, but close natively without waiting.

Unmounting a `<window>` from app code (without a user close) closes its native window/tab the same
way — the remove rides a `window.close` semantic action, so JS-initiated and user-initiated closes
converge on one path.

## Tracking the frontmost tab

Every `<window>` also fires `onFocused({ checked })` when it gains or loses key/active status — for
a plain window that's a normal focus change, but for a `tabGroup` member it also fires when the
user switches native tabs: the outgoing tab gets `checked: false`, the incoming one `checked: true`.
This is the signal to keep for app-level state (menu items, keyboard shortcuts, a command palette)
that needs to act on "whichever tab is frontmost right now" rather than on whatever tab last
rendered — without it, an app tracking its own "selected" value has no way to learn that the user
switched tabs natively, since the switch never touches React.

```tsx
<window
  title="Terminal"
  tabGroup="terminal"
  onFocused={({ checked }) => {
    if (checked) setActiveTabId(id);
  }}
>
  {/* … */}
</window>
```

The examples bind a `New Tab` menu item (`accelerator="primary+t"`) to the same handler as
`onNewTabRequested`, which is the conventional Cmd+T/Ctrl+T affordance on both platforms.

## The tab overview

Both platforms have a native tab overview. On GNOME the framework wraps each group window's content
in an `AdwTabOverview` and packs an `AdwTabButton` (with its live page-count badge) at the end of
your `<headerbar>`; on macOS it's the system Show All Tabs (⇧⌘\, View menu, or pinch on the tab
bar). To open it programmatically:

```tsx
import { showTabOverview } from "@nativedesktop/react";

showTabOverview(winRef.current); // AdwTabOverview open on GTK, toggleTabOverview on macOS
```

## Drag and drop, Chrome-style

Dragging tabs out to the desktop, into another window of the same group, and reordering are all
native on both backends — and none of it touches your React tree. The `<window>` node's identity is
stable across drags: on macOS the tab IS its `NSWindow`, so moving it between groups moves the live
window; on GTK the framework spawns a fresh scaffold window on `create-window` and `AdwTabView`
transfers the page widget intact. A `<webview>` tab keeps its session and a `<terminal>` tab keeps
its running shell through any drag. Empty windows left behind by a drag close themselves.

Where a new tab opens follows the platform convention: the group's most recently focused window.

## Platform notes

- The tab bar autohides with a single tab on GTK (`AdwTabBar` autohide); macOS shows it once a
  second tab joins (or via View → Show Tab Bar).
- On GTK the tab bar slots INTO your `<toolbarview>` below its `<headerbar>` (the Epiphany layout);
  a window without a `<toolbarview>` root gets a framework wrapper carrying the tab bar. Per-tab
  headerbars swap with the active tab — the same semantics macOS gets from per-window toolbars.
- `tabGroup` is create-time only: a window can't move between groups by prop update. Different
  groups are fully independent — a "browser" tab never merges into a "terminal" window.
- GNOME's tab widgets require libadwaita ≥ 1.4, which the toolkit already assumes for
  `AdwToolbarView`.
