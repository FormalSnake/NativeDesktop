---
title: System Capabilities
description: Native file dialogs, clipboard, notifications, recent documents, credentials, live appearance, and app-level OS events, with one API and real native behavior on each backend.
---

`@nativedesktop/react` exposes the OS-level surface a desktop app eventually needs as promise-based
calls: file pickers, clipboard, notifications, Open Recent, the system credential store, audio
playback, live light/dark and accent color, and app lifecycle events like activation and file drops.
Every call runs the real native API on the host (`NSOpenPanel`/`GtkFileDialog`,
`NSPasteboard`/`GdkClipboard`, `UNUserNotificationCenter`/`GNotification`, Keychain and Secret
Service, `AVPlayer` and GStreamer) from the same app code on both backends.

## Dialogs

```tsx
import { dialog } from "@nativedesktop/react";

async function openMarkdownFile() {
  const paths = await dialog.openFile({
    title: "Open Document",
    filters: [{ name: "Markdown", extensions: ["md", "markdown"] }],
  });
  if (paths.length === 0) return; // user canceled
  console.log("opened", paths[0]);
}
```

| Method | Resolves to | Cancel behavior |
| --- | --- | --- |
| `dialog.openFile(options?)` | `string[]` | `[]` |
| `dialog.saveFile(options?)` | `string \| null` | `null` |
| `dialog.showMessage(options)` | `number` (clicked button index) | resolves to `defaultButton` |

`openFile`/`saveFile` share `title`, `defaultPath`, and `filters: { name, extensions: string[] }[]`;
`openFile` adds `multiple` and `directories`, `saveFile` adds `defaultName`. `showMessage` takes
`message`, an optional `detail`, `level: "info" | "warning" | "error"`, and `buttons` (defaults to
`["OK"]`); it resolves to the 0-based index of the clicked button.

These are app-level dialogs backed by `NSOpenPanel`, `NSSavePanel`, and `NSAlert` on macOS,
`GtkFileDialog` and `GtkAlertDialog` on GTK. They are a different mechanism from the `<window>`
widget's own `showAlert`/`openFile`/`saveFile`/`showAbout` imperative commands (see
[Dialogs](/components/dialogs/)). Use `dialog.*` unless you need a dialog scoped to one window's
command channel, or the About panel, which only the `<window>` version exposes.

## Clipboard

```tsx
import { clipboard } from "@nativedesktop/react";

<button label="Copy link" onClick={() => clipboard.writeText("https://example.com/")} />;
```

`clipboard.writeText(text)` is default-granted. `clipboard.readText()` resolves `""` when the
clipboard holds no text, but reading the clipboard is default-denied; see
[Permissions](#permissions).

## Notifications

```tsx
import { notifications, useEffect } from "@nativedesktop/react";

function useNotifyOnDone(done: boolean, runId: string) {
  useEffect(() => {
    const unsubscribe = notifications.onClick((e) => {
      const clicked = (e.data ?? {}) as { runId?: string };
      if (clicked.runId) revealRun(clicked.runId);
    });
    return unsubscribe;
  }, []);

  if (done) notifications.show({ title: "Build finished", body: "0 errors, 0 warnings", data: { runId } });
}
```

`notifications.show({ title, body?, data? })` resolves to a notification id.
`notifications.onClick(handler)` fires `{ id, data? }` when the user clicks the banner, where `data`
is whatever you passed to `show()`. The payload lives in a process-local map, never sent to the
host, capped at 128 entries and cleared per notification once its click dispatches. It survives
`bun --hot` re-evals but not an app restart, which costs nothing today: a click arriving after a
restart is dropped by the transport anyway, the same gap as `onOpenUrl` below. `onClick` returns an
unsubscribe function, so it composes with a `useEffect` cleanup.

## Recent documents

```tsx
import { recentDocuments } from "@nativedesktop/react";

await recentDocuments.add("/Users/me/notes.md");
```

`recentDocuments.add(path)` and `recentDocuments.clear()` drive `NSDocumentController`'s "Open
Recent" menu on macOS and `GtkRecentManager` on GTK.

## Credentials

```tsx
import { credentials } from "@nativedesktop/react";

await credentials.set("my-app", "api-token", secretValue);
const token = await credentials.get("my-app", "api-token"); // null if not found
await credentials.delete("my-app", "api-token");
```

`credentials.set/get/delete` store secrets by `(service, account)` in the OS credential store:
Keychain on macOS, Secret Service (via `libsecret`) on GTK. Like clipboard reads, credential access
is default-denied.

## Audio

```tsx
import { audio, useEffect, useRef, useState } from "@nativedesktop/react";

function PlayerWithMeter({ path }: { path: string }) {
  const handle = useRef<string | null>(null);
  const [level, setLevel] = useState(0);

  useEffect(() => {
    const offSpectrum = audio.onSpectrum((e) => {
      if (e.handle === handle.current) setLevel(Math.max(...e.bins));
    });
    const offState = audio.onState((e) => {
      if (e.handle === handle.current && (e.state === "ended" || e.state === "error")) {
        handle.current = null;
        setLevel(0);
      }
    });
    return () => {
      offSpectrum();
      offState();
      if (handle.current) audio.stop(handle.current);
    };
  }, []);

  return (
    <box orientation="vertical" spacing={8}>
      <button
        label="Play"
        onClick={async () => {
          handle.current = await audio.play({ path, volume: 0.8, spectrum: true });
        }}
      />
      <progressbar fraction={level} />
    </box>
  );
}
```

`audio.play(options)` starts playback immediately and resolves to a string **handle** the other
methods take. `options` must name exactly one source, `path` (a local file) or `url` (a remote
stream), plus an optional `volume` (0..1, default 1) and `spectrum: true` if you want spectrum
frames for this playback.

| Method | Effect |
| --- | --- |
| `audio.pause(handle)` / `audio.resume(handle)` | Pause / resume playback. |
| `audio.stop(handle)` | Stops playback and releases the handle; it's invalid afterward. |
| `audio.seek(handle, positionMs)` | Seeks to a position in milliseconds. |
| `audio.setVolume(handle, volume)` | Sets volume (0..1). |

All control methods resolve `void`; calling any of them with an unknown (or already-stopped) handle
rejects with `"unknown audio handle"`.

There are two event subscriptions; each returns an unsubscribe function like the `app.on*` family:

- **`audio.onState(handler)`** fires on playback transitions only; there are no position ticks.
  The event is `{ handle, state, position, duration, error? }` with `state` one of
  `"playing" | "paused" | "ended" | "stopped" | "error"`; `position`/`duration` are in milliseconds,
  and `duration` is `null` until the media's length is known. Synchronous failures (bad params, a
  missing local file) reject the `play()` promise itself; asynchronous media failures (bad codec, an
  unreachable URL) arrive later as a `state: "error"` event with the message in `error`.
- **`audio.onSpectrum(handler)`** fires at roughly 15 Hz with `{ handle, bins }`: 32 magnitudes
  normalized 0..1, log-spaced across roughly 50 Hz to 16 kHz. It fires only for handles played with
  `spectrum: true`.

## Appearance

```tsx
import { system, useEffect, useState } from "@nativedesktop/react";

function AccentDot(): React.ReactNode {
  const [info, setInfo] = useState({ appearance: "light", accentColor: "#0066cc" });
  useEffect(() => {
    system.getAppearance().then(setInfo);
    return system.onAppearanceChange(setInfo);
  }, []);
  return <box style={{ background: info.accentColor }} />;
}
```

`system.getAppearance()` resolves `{ appearance: "light" | "dark", accentColor: "#rrggbb" }` and
`system.onAppearanceChange(handler)` subscribes to changes in either. The accent is
`AdwStyleManager`'s on GTK, `NSColor.controlAccentColor` on macOS. Use it when you need the live
accent for something dynamic, such as a status dot or a chart series, instead of a hardcoded hex.
Light and dark itself needs no polling: unstyled widgets and `cssClasses` already track the system.
See [Styling & Design Language](/core-concepts/styling-design-language/#dark-mode-is-automatic).

## App-level events

```tsx
import { app, useEffect } from "@nativedesktop/react";

function useFileDrop(onFiles: (paths: string[]) => void) {
  useEffect(() => {
    return app.onFileDrop((e) => onFiles(e.paths));
  }, [onFiles]);
}
```

`app.isActive()` answers the standing question synchronously (no await, no subscription), backed
by the same activation stream. The host replays the current state right after the NDP handshake, so
it's already correct in your first render (and after an HMR or crash respawn).

| Subscription | Fires when… |
| --- | --- |
| `app.onActivate(h)` / `app.onDeactivate(h)` | The whole app gains/loses focus (Dock/taskbar re-activation). |
| `app.onOpenUrl(h: (url) => void)` | The OS delivers a launch for a registered URL scheme. |
| `app.onOpenFile(h: (paths: string[]) => void)` | The OS delivers a launch for a registered file association (e.g. double-clicking a document). |
| `app.onFileDrop(h: (e: { paths: string[]; windowId: number }) => void)` | Files are dragged onto an app window. `windowId` is currently always `0`. |

Every subscription returns an unsubscribe function, so it composes directly with a `useEffect`
cleanup. `onOpenUrl`/`onOpenFile` events fired before the app's first render can be missed (there's
no buffering yet), so register these as early as possible. See [Packaging](/packaging/) for how
`fileAssociations`/`urlSchemes` get registered with the OS in the first place.

## Shell helpers

```tsx
import { openExternal, openPath, revealPath } from "@nativedesktop/react";

await openExternal("https://example.com/");   // OS default browser
await openPath("/Users/me/notes.md");          // OS default app for the file
await revealPath("/Users/me/notes.md");        // reveal + select in Finder/file manager
```

`openExternal`, `openPath`, and `revealPath` are plain TypeScript. They spawn `open` or `xdg-open`
directly in the app's Bun process (`revealPath` on Linux uses the freedesktop `FileManager1` D-Bus
interface, falling back to `xdg-open` on the containing directory). They do not round-trip through
the host and are not ACL-gated, because the Bun child is already a full unsandboxed runtime. See
[Architecture](/core-concepts/architecture/).

## Permissions

Every `dialog.*`/`clipboard.*`/`notification.*`/`recent.*`/`credentials.*`/`audio.*`/`system.*` call
is gated host-side by the same capability ACL that guards widget commits. Some groups are granted by
default; the rest reject with `Error("capability denied")` until the app's host process is started
with an explicit grant.

| Group | Default | Covers |
| --- | --- | --- |
| `core:dialog` | granted | `dialog.openFile`, `dialog.saveFile`, `dialog.showMessage` |
| `core:notification` | granted | `notifications.show` |
| `core:recent` | granted | `recentDocuments.add`, `recentDocuments.clear` |
| `core:clipboard.write` | granted | `clipboard.writeText` |
| `core:audio` | granted | all `audio.*` calls |
| `core:system` | granted | `system.getAppearance`, `system.onAppearanceChange` |
| `core:clipboard.read` | **denied** | `clipboard.readText` |
| `core:credentials` | **denied** | `credentials.set`, `credentials.get`, `credentials.delete` |

Grant the denied groups by setting `ND_ACL_GRANTS` on the host process. It takes a JSON object with a
`defaultWindow` array (applies to every window) and/or a `grants` array of `{ window, permissions }`
entries for per-window grants:

```bash
ND_ACL_GRANTS='{"defaultWindow":["core:clipboard.read","core:credentials"]}' nd dev
```

A denied call rejects its promise with `Error("capability denied")`, and the host logs
`ND_ACL_DENY permission=core:clipboard.read` (substituting the denied group) so a rejected call is
easy to trace back to the missing grant.

## How it works

Every `dialog`, `clipboard`, `notification`, `recentDocuments`, `credentials`, `system`, and `audio`
call sends an id-correlated `systemRequest` NDP frame to the host. The host resolves the method to a
`core:*` capability, runs the ACL check, and for an allowed request runs the real native API on the
UI thread before replying with a `systemResponse` frame that settles the promise. `app.on*`
subscriptions receive host-initiated `systemEvent` frames instead, pushed whenever the OS delivers
an activation, launch, or file drop; `audio.onState` and `audio.onSpectrum` ride the same channel.
The shell helpers never touch this path.

## Platform notes

- **macOS notifications and bundling.** A packaged `.app` delivers notifications through
  `UNUserNotificationCenter`, which requests banner/sound authorization on first use and delivers
  click events through `notifications.onClick`. The bare (unbundled) `nd dev` process has no bundle
  identifier, so it falls back to the deprecated `NSUserNotificationCenter` API instead. Test click
  delivery against a packaged build rather than the dev shell.
- **GTK message levels.** `dialog.showMessage`'s `level` is accepted on GTK but has no visual
  effect, since `GtkAlertDialog` has no per-severity styling. Dismissing the dialog with Escape
  resolves to `defaultButton` on both backends.
- **Linux credentials need libsecret.** `credentials.*` on GTK dlopens `libsecret-1.so` at runtime
  (never a build-time link) and rejects with a clean "credential store unavailable" error if it isn't
  installed, rather than failing to build or crashing.
- **macOS audio.** Playback rides `AVPlayer`, and local files and remote URLs go through the same
  code path. Spectrum analysis hangs an audio tap off the player item, so `spectrum: true` works for
  both source kinds with no extra setup.
- **Linux audio needs GStreamer.** Like libsecret, GStreamer is loaded at runtime rather than
  linked: if it isn't installed, every `audio.*` call rejects with
  `"audio unavailable: gstreamer not found"`. If GStreamer is present but its `spectrum` plugin is
  missing, playback still works; spectrum events just never arrive (the host logs a warning).
