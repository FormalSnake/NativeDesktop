---
title: Quick Start
description: From an empty directory to a running native window in under five minutes.
---

You write SolidJS in TypeScript. A native host process renders it as real platform widgets: AppKit
on macOS, GTK4 with libadwaita on Linux. This page takes you from an empty directory to a running
window.

## Create a project

```bash
mkdir hello-native && cd hello-native
bun add @nativedesktop/cli @nativedesktop/solid solid-js
bun add -d typescript @types/bun
```

Add a `dev` script to the generated `package.json`:

```json
{
  "scripts": {
    "dev": "nd dev"
  }
}
```

Create a `tsconfig.json`. `jsx: "preserve"` leaves the JSX to Solid's compiler, and
`jsxImportSource` types it against NativeDesktop's widgets.

```json
{
  "compilerOptions": {
    "target": "ESNext",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "lib": ["ESNext"],
    "jsx": "preserve",
    "jsxImportSource": "@nativedesktop/solid",
    "strict": true,
    "skipLibCheck": true,
    "noEmit": true,
    "types": ["bun"]
  },
  "include": ["**/*.ts", "**/*.tsx"]
}
```

Bun does not compile Solid JSX on its own. `nd dev`, `nd build` and `nd package` preload
`@nativedesktop/solid/register`, which runs the `.tsx` files of any package that depends on
`@nativedesktop/solid` through Solid's compiler. There is nothing to configure.

There is no scaffolding command for a published install yet; these files are the whole setup. In a
framework checkout, `./scripts/new-app.sh ../my-app` copies `template/`.

## First component

Create `src/main.tsx`, the default entry point for `nd dev`:

```tsx
import { render } from "@nativedesktop/solid";

function App() {
  return (
    <window title="Hello" defaultWidth={480} defaultHeight={320}>
      <box orientation="vertical" spacing={8}>
        <label text="Hello from Solid" />
      </box>
    </window>
  );
}

await render(() => <App />);
```

`<window>`, `<box>`, and `<label>` are NativeDesktop intrinsics. Each one is a real native widget:
`NSWindow` and `NSTextField` on macOS, `AdwApplicationWindow` and `GtkLabel` on Linux. There is no
DOM anywhere.

## Run it

```bash
bun run dev
```

A native window opens. The terminal prints `ND_CHILD_CONNECTED` when your Solid process attaches to
the host, then `ND_COMMIT_APPLIED` for every commit it ships across.

## Add state

State is a Solid signal. Replace `src/main.tsx`:

```tsx
import { createSignal } from "solid-js";
import { render } from "@nativedesktop/solid";

function App() {
  const [clicks, setClicks] = createSignal(0);

  return (
    <window title="Hello" defaultWidth={480} defaultHeight={320}>
      <box orientation="vertical" spacing={8}>
        <label text={`Clicks: ${clicks()}`} />
        <button label="Increment" onClick={() => setClicks((c) => c + 1)} />
      </box>
    </window>
  );
}

await render(() => <App />);
```

`App` runs once. Reading `clicks()` inside the `text` expression subscribes that one prop, so a
click updates the label and nothing else re-runs. See
[State & Hot Reload](/core-concepts/state-hot-reload/) for what an edit preserves.

## Hot reload

Leave `bun run dev` running. Click the button a few times, then change the label text in
`src/main.tsx` and save. The window updates in place and the click count survives the edit: Solid's
refresh runtime patches the edited component and leaves the rest of the live tree mounted.

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

## Next

- [Build a Counter](/get-started/tutorial-counter/): components, state, and native styling classes.
- [Build a Settings Window](/get-started/tutorial-settings/): sidebar navigation, settings rows, persistence, dialogs.
- [Build a Tabbed Terminal](/get-started/tutorial-terminal/): the `<terminal>` widget and native system tabs.
- [App Model](/core-concepts/app-model/): how a window and its chrome are built from JSX.
