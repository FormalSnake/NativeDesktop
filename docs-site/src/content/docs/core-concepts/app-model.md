---
title: App Model
description: How a NativeDesktop app is structured as a JSX tree rooted at a window.
---

A NativeDesktop app is a React tree rooted at a `<window>` intrinsic, rendered with `render()` from
`@nativedesktop/react`:

```tsx
import { render, useState } from "@nativedesktop/react";

function App(): React.ReactNode {
  const [clicks, setClicks] = useState(0);
  return (
    <window title="My App" defaultWidth={480} defaultHeight={320}>
      <box orientation="vertical" spacing={8}>
        <label text={`Clicks: ${clicks}`} />
        <button label="Increment" onClick={() => setClicks((c) => c + 1)} />
      </box>
    </window>
  );
}

await render(<App />);
```

An app can also mount more than one `<window>` at once — render several `<window>` roots (e.g. in a
fragment) and each becomes an independent OS window. See [Multi-Window](/native-platform/multi-window/)
for the details and for how to move a live widget between windows without reloading it.

## Chrome is declarative

Native chrome — headerbars, toolbars, split views — is composed the same way as any other widget:
declared as JSX children, not configured through an imperative window API. `examples/notes/main.tsx`
builds a two-pane app entirely out of intrinsics:

```tsx
<window title="ND Notes" defaultWidth={900} defaultHeight={600}>
  <splitview sidebarWidth={0.28}>
    <toolbarview slot="sidebar">
      <headerbar>
        <button iconName="document-new" onClick={createNote} slot="start" />
      </headerbar>
      {/* sidebar content */}
    </toolbarview>
    <toolbarview slot="content">
      <headerbar title={selected?.title ?? "ND Notes"} />
      {/* content pane */}
    </toolbarview>
  </splitview>
</window>
```

`slot` here is an **attached prop** — set on a child to tell its container widget where that child
belongs (`sidebar`/`content` for `<splitview>`, `start`/`end` for `<headerbar>`). Attached props are
attach-time-only: changing one after mount is a no-op. See
[Windows & Chrome](/native-platform/windows-chrome/) for how these compose on each platform today.

## Events are props

Widget events arrive as ordinary React props — `onClick`, `onChanged`, `onToggled`,
`onSelectionChanged`, and so on — each wired from the schema's `events` list for that widget.
`<textinput onChanged={(e) => setText(e.text)} />` receives the new text on its event payload,
exactly like any other controlled-component callback.

## Create-only vs. createAndUpdate props

Not every prop can be changed after a widget mounts. The schema marks each prop's `appliesTo` as
`create` (set once, at construction), `createAndUpdate` (can change on any re-render), or `meta`
(framework bookkeeping, e.g. `testID`). `Button.label`, for example, is `create`-only — changing it
on a live button is a no-op, which is why `examples/notes/main.tsx` keys its header on the note's
`id`/`title` to force a remount instead of relying on a prop update. Check a widget's `Applied`
column before assuming a prop is live-updatable — see the full breakdown in the
[Widget Reference](/components/widget-reference/).
