# Runtime dependencies

What a machine needs at run time to launch a NativeDesktop app from the
published packages. Nothing here is needed at build time by app authors; the
host binaries ship prebuilt in `@nativedesktop/host-darwin-arm64` and
`@nativedesktop/host-linux-x64`.

## macOS (nd-shell)

Self-contained. The binary links only Apple system frameworks, plus the
statically linked `libnd.a` core and libghostty-vt archive. macOS 15 or newer
on Apple silicon.

## Linux (nd-hello)

The GTK host links the platform toolkit dynamically by soname; it is built in
an ubuntu:26.04 container. The hard code-level requirement is libadwaita 1.7
(AdwToggleGroup, the accent-color API); the glibc and GTK floors are whatever
that container ships, recorded per release in the attached nd-hello.ldd.txt
and the release job's "Print library floors" step. Static linking is not an
option for this stack: libadwaita
needs GSettings schemas, GIO modules, icon themes, and gdk-pixbuf loaders from
the host system.

### Required (the app does not start without these)

| Library | Debian/Ubuntu package | Fedora package |
|---|---|---|
| `libgtk-4.so.1` (>= 4.14) | `libgtk-4-1` | `gtk4` |
| `libadwaita-1.so.0` (>= 1.7) | `libadwaita-1-0` | `libadwaita` |
| `libglib-2.0.so.0`, `libgobject-2.0.so.0`, `libgio-2.0.so.0` | `libglib2.0-0` | `glib2` |
| `libpango-1.0.so.0`, `libpangocairo-1.0.so.0` | `libpango-1.0-0` | `pango` |
| `libcairo.so.2`, `libcairo-gobject.so.2` | `libcairo2` | `cairo` |
| `libgdk_pixbuf-2.0.so.0` | `libgdk-pixbuf-2.0-0` | `gdk-pixbuf2` |
| `libgraphene-1.0.so.0` | `libgraphene-1.0-0` | `graphene` |
| `libharfbuzz.so.0` | `libharfbuzz0b` | `harfbuzz` |
| `libutil` (forkpty, part of glibc) | `libc6` | `glibc` |
| glibc (floor set by the ubuntu:26.04 build container; see the release ldd listing) | `libc6` | `glibc` |

Installing the distro's libadwaita package pulls in everything above on any
mainstream distribution.

### Optional (dlopen'd by bare soname; the feature degrades if absent)

| Library | Enables | Without it |
|---|---|---|
| `libwebkitgtk-6.0.so.4` (+ `glib-networking` for TLS) | `<webview>` | webview area renders an unavailable placeholder |
| `libgstreamer-1.0.so.0` + gst-plugins-base/-good | `audio.*` playback and spectrum | audio calls reject with "audio unavailable" |
| `libsecret-1.so.0` | `credentials.*` | credential calls reject |

### How to check a machine

```bash
# every line must resolve; "not found" names the missing package
ldd node_modules/@nativedesktop/host-linux-x64/bin/nd-hello

# the optional sonames, individually
ldconfig -p | grep -E 'libwebkitgtk-6.0.so.4|libgstreamer-1.0.so.0|libsecret-1.so.0'
```

The release workflow records the full `ldd` output of every published binary
as the `nd-hello.ldd.txt` asset on the GitHub release.
