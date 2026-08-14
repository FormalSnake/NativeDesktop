---
title: Video
description: "The <video> widget plays local or remote video through the platform's own media stack: AVPlayer with AVPlayerView on macOS, GtkVideo on Linux."
---

`<video>` plays a local file or a remote URL through the platform's own media stack: `AVPlayerView`
and `AVPlayer` on macOS, `GtkVideo` on GTK. No engine is bundled, same as `<webview>` and the
[Audio API](/native-platform/system-capabilities/#audio).

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
| `controls` | bool | create | Default `true`. Takes effect on macOS only, see below. |

There are no events yet (`onEnded`, `onTimeUpdate`, `onError`). For playback state, track it
against the [Audio API](/native-platform/system-capabilities/#audio)'s richer event set.

## Platform notes

- `controls` takes effect on macOS only. `AVPlayerView` toggles its overlaid transport controls
  from the prop; `GtkVideo`'s overlaid controls have no such toggle and always show. The prop is
  accepted and typed on both backends.
- GTK needs a media backend installed. `GtkVideo` renders through GTK's pluggable media-file
  backend, for example `gtk4-media-gstreamer`, which some distros package separately from GTK. With
  none installed, `<video>` shows a placeholder label reading "Video unavailable (GTK media module
  not installed)" and logs `ND_WARN Video unavailable`, the same degrade path
  [`<webview>`](/components/webview/) takes when `webkitgtk` is missing.
- macOS has no such dependency. `AVPlayerView` is a system framework.

See `examples/gallery/main.tsx`'s Video tab, which swaps `src` between a bundled sample clip and
whatever file `openFile()` (see [Dialogs](/components/dialogs/)) returns.
