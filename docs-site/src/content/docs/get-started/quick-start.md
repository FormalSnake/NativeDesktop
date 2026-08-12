---
title: Quick Start
description: From an empty directory to a running native window in under five minutes.
---

You write React in TypeScript. A native host process renders it as real platform widgets: AppKit on
macOS, GTK4 with libadwaita on Linux. This page takes you from an empty directory to a running
window.

## Create a project

```bash
mkdir hello-native && cd hello-native
bun add @nativedesktop/cli @nativedesktop/react react
bun add -d typescript @types/react @types/bun
```

Add a `dev` script to the generated `package.json`:

```json
{
  "scripts": {
    "dev": "nd dev"
  }
}
```

Create a `tsconfig.json`. The `jsxImportSource` line matters twice: it types the JSX against
NativeDesktop's widgets, and Bun reads it to transpile your JSX at run time.

```json
{
  "compilerOptions": {
    "target": "ESNext",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "lib": ["ESNext", "DOM"],
    "jsx": "react-jsx",
    "jsxImportSource": "@nativedesktop/react",
    "strict": true,
    "skipLibCheck": true,
    "noEmit": true,
    "types": ["bun", "react"]
  },
  "include": ["**/*.ts", "**/*.tsx"]
}
```

There is no scaffolding command yet; these three files are the whole setup.

## First component

Create `src/main.tsx`, the default entry point for `nd dev`:

```tsx
import { render } from "@nativedesktop/react";

function App() {
  return (
    <window title="Hello" defaultWidth={480} defaultHeight={320}>
      <box orientation="vertical" spacing={8}>
        <label text="Hello from React" />
      </box>
    </window>
  );
}

await render(<App />);
```

`<window>`, `<box>`, and `<label>` are NativeDesktop intrinsics. Each one is a real native widget:
`NSWindow` and `NSTextField` on macOS, `AdwApplicationWindow` and `GtkLabel` on Linux. There is no
DOM anywhere.

## Run it

```bash
bun run dev
```

A native window opens. The terminal prints `ND_CHILD_CONNECTED` when your React process attaches to
the host, then `ND_COMMIT_APPLIED` for every commit React ships across.

## Add state

State is plain React. Replace `src/main.tsx`:

```tsx
import { render, useState } from "@nativedesktop/react";

function App() {
  const [clicks, setClicks] = useState(0);

  return (
    <window title="Hello" defaultWidth={480} defaultHeight={320}>
      <box orientation="vertical" spacing={8}>
        <label text={`Clicks: ${clicks}`} />
        <button label="Increment" onClick={() => setClicks((c) => c + 1)} />
      </box>
    </window>
  );
}

await render(<App />);
```

Import hooks from `@nativedesktop/react`, not from `react`. Hot reload re-evaluates the whole module
graph, and a bare `react` import would resolve to a fresh module instance with no attached
dispatcher. See [State & Hot Reload](/core-concepts/state-hot-reload/) for the mechanics.

## Hot reload

Leave `bun run dev` running. Click the button a few times, then change the label text in
`src/main.tsx` and save. The window updates in place and the click count survives the edit:
`react-refresh` patches the live component tree instead of remounting it.

If your code throws, the window stays up. The host owns the native process, so a JS crash shows an
error overlay with a Restart button instead of taking the window down.

## Pick a backend

`nd dev` picks the native backend for your platform: AppKit on macOS, GTK on Linux. The
`--backend` flag and the `ND_BACKEND` env var override it:

```bash
bunx nd dev --backend gtk
```

From an npm install this only matters inside the framework's source checkout, where macOS can
cross-check the GTK host through its Quartz backend. Prebuilt binaries ship one host per platform,
so `--backend gtk` on a macOS npm install fails with a resolution error.

## Where to go next

- [Build a Counter](/get-started/tutorial-counter/): components, state, and native styling classes.
- [Build a Settings Window](/get-started/tutorial-settings/): sidebar navigation, settings rows, persistence, dialogs.
- [Build a Tabbed Terminal](/get-started/tutorial-terminal/): the `<terminal>` widget and native system tabs.
- [App Model](/core-concepts/app-model/): how a window and its chrome are built from JSX.
