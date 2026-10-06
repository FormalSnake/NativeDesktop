---
title: App Model
description: How a NativeDesktop app is structured as a JSX tree rooted at a window.
---

A NativeDesktop app is a Solid component tree rooted at a `<window>` intrinsic, rendered with `render()` from
`@nativedesktop/solid`:

```tsx
import { render } from "@nativedesktop/solid";
import { createSignal } from "solid-js";

function App() {
  const [clicks, setClicks] = createSignal(0);
  return (
    <window title="My App" defaultWidth={480} defaultHeight={320}>
      <box orientation="vertical" spacing={8}>
        <label text={`Clicks: ${clicks()}`} />
        <button label="Increment" onClick={() => setClicks((c) => c + 1)} />
      </box>
    </window>
  );
}

await render(() => <App />);
```

An app can mount more than one `<window>` at once. Render several `<window>` roots, for example in a
fragment, and each becomes an independent OS window. See
[Multi-Window](/native-platform/multi-window/) for the details, including how to move a live widget
between windows without reloading it.

## Chrome is declarative

Native chrome (headerbars, toolbars, split views) is composed the same way as any other widget:
declared as JSX children rather than configured through an imperative window API.
`examples/notes/main.tsx` builds a two-pane app entirely out of intrinsics:

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
      <headerbar title={selected()?.title ?? "ND Notes"} />
      {/* content pane */}
    </toolbarview>
  </splitview>
</window>
```

`slot` here is an attached prop, set on a child to tell its container widget where that child
belongs: `sidebar` or `content` for `<splitview>`, `start` or `end` for `<headerbar>`. Attached
props apply at attach time only, so changing one after mount is a no-op. See
[Windows & Chrome](/native-platform/windows-chrome/) for how these compose on each platform today.

## Events are props

Widget events arrive as ordinary props (`onClick`, `onChanged`, `onToggled`,
`onSelectionChanged`, and so on), each wired from the schema's `events` list for that widget.
`<textinput onChanged={(e) => setText(e.text)} />` receives the new text on its event payload,
exactly like any other event callback.

## Create-only vs. createAndUpdate props

Not every prop can be changed after a widget mounts. The schema marks each prop's `appliesTo` as
`create` (set once, at construction), `createAndUpdate` (follows its signal on any change), or `meta`
(framework bookkeeping, e.g. `testID`). `Label.ellipsize`, for example, is `create`-only: changing
it on a live label is a no-op, so a label that needs a new value remounts under
`<Show when={mode()} keyed>{(m) => <label ellipsize={m === "short"} text="..." />}</Show>`
instead of relying on a prop update. (`Label.text` is also marked `create`, but the Solid renderer
sends text changes as `setText` ops, so a reactive `text` stays live.)
Check a widget's `Applied` column before assuming a
prop is live-updatable; the full breakdown is in the
[Widget Reference](/components/widget-reference/).
