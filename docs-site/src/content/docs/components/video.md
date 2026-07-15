---
title: Video
description: The <video> widget plays local or remote video through the platform's own media stack — AVPlayer/AVPlayerView on macOS, GtkVideo on Linux.
---

`<video>` plays a local file or remote URL through the platform's own media stack —
`AVPlayerView`/`AVPlayer` on macOS, `GtkVideo` on GTK — the same "no bundled engine" contract
`<webview>` and the [Audio API](/native-platform/system-capabilities/#audio) follow.

```tsx
const [src, setSrc] = useState("/Users/me/clip.mp4");

<video src={src} controls loop style={{ vexpand: true }} />;
```

## Props

| Prop | Type | Applied | Notes |
| --- | --- | --- | --- |
| `src` | string | createAndUpdate | A local path or a remote URL. |
| `autoplay` | bool | create | Default `false`. |
| `loop` | bool | createAndUpdate | Default `false`. |
| `controls` | bool | create | Default `true`. **GTK-only in effect** — see below. |

No events yet — there's no `onEnded`/`onTimeUpdate`/`onError` surface in v1; if you need playback
state, drive audio-visual sync yourself or track it against the [Audio API](/native-platform/system-capabilities/#audio)'s
richer event set instead.

## Platform notes

- **`controls` only takes effect on macOS.** `AVPlayerView` toggles its overlaid transport controls
  from the prop; **`GtkVideo`'s overlaid controls have no such toggle**, so on GTK they always show
  regardless of what `controls` is set to. The prop is still accepted (and typed) on both backends —
  it just has no visible effect on Linux.
- **GTK needs a media backend installed.** `GtkVideo` renders through GTK's pluggable media-file
  backend (e.g. `gtk4-media-gstreamer`), which some distros package separately from GTK itself. If
  none is installed, `<video>` **degrades to a placeholder label** — "Video unavailable (GTK media
  module not installed)" — and logs `ND_WARN Video unavailable`, the same degrade-don't-fail pattern
  [`<webview>`](/components/webview/) uses when `webkitgtk` is missing. An app that uses `<video>`
  still builds and runs everywhere; it just shows no picture where the media module is absent.
- **macOS has no such dependency** — `AVPlayerView` is a system framework, always present.

See `examples/gallery/main.tsx`'s "Video" tab, which swaps `src` between a bundled sample clip and
whatever file `openFile()` (see [Dialogs](/components/dialogs/)) returns.
