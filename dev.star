# The same environment as flake.nix's devShell, for fspec dev and `use fspec`.

# rust-toolchain.toml is resolved by rustup's proxies, which act on the name they run as.
RUST = ["cargo", "rustc", "rustdoc", "rustfmt", "cargo-fmt", "cargo-clippy", "clippy-driver"]

# GPUI links and dlopens these; libcef.so links the rest.
GPU = ["wayland-client", "xkbcommon", "vulkan", "x11", "xcb", "xcursor", "xi", "xrandr", "xkbfile",
       "fontconfig", "freetype2", "alsa", "openssl", "gbm", "egl"]
CEF = ["glib-2.0", "nss", "nspr", "atk", "atk-bridge-2.0", "libdrm", "expat", "gtk+-3.0", "pango",
       "cairo", "dbus-1", "atspi-2", "cups", "gl", "libudev", "libsystemd", "xcomposite", "xdamage",
       "xext", "xfixes", "xshmfence"]
# nd-system's audio and the GPUI backend's video() dlopen GStreamer: playbin, appsink,
# videoconvert and videoscale (base), qtdemux, videoflip, spectrum and the PulseAudio sink
# (good), the H.264 and HEVC decoders (libav, and bad's VA-API ones). Plugins ship no pkg-config
# module, so they are named by their shared object.
GST = ["gstreamer-1.0", "gstreamer-plugins-base-1.0", "gstreamer-plugins-bad-1.0"] + [
    "gstreamer-1.0/libgst%s.so" % p
    for p in ["playback", "app", "videoconvertscale", "isomp4", "videofilter", "spectrum", "pulseaudio", "libav", "va"]
]

# Goldens draw the Adwaita preset's fonts and the desktop's interface fonts, so every host sees
# the same few.
FONTS = [
    pkg("adwaita-fonts", source = url("https://download.gnome.org/sources/adwaita-fonts/51/adwaita-fonts-51.0.tar.xz")),
    pkg("noto-color-emoji", source = url("https://github.com/googlefonts/noto-emoji/raw/v2.051/fonts/NotoColorEmoji.ttf")),
]

linux = platform.is_linux

def uniq(names):
    out = []
    for n in names:
        if n not in out:
            out.append(n)
    return out

COMMON_HOOK = """
unset RUSTC CARGO RUSTDOC
# Several agents build here at once next to the user's own services; cap each build and test
# run. Workspace-wide and Linux-capable runs belong on g815 (scripts/linux-gate).
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
export NEXTEST_TEST_THREADS="${NEXTEST_TEST_THREADS:-4}"
"""

DARWIN_HOOK = """
unset SDKROOT DEVELOPER_DIR MACOSX_DEPLOYMENT_TARGET
# The system toolchain in /usr/bin must win over a nix cc, xcrun or strip in the calling PATH.
export PATH="/usr/bin:$PATH"
# nd-cef's build script downloads the CEF binary distribution (about 500 MB unpacked) into
# $CEF_PATH; on the macbook that is the cache drive, next to the cargo build dir.
if [ -z "${CEF_PATH:-}" ]; then
  if [ -d "/Volumes/Kyan Drive" ]; then
    export CEF_PATH="/Volumes/Kyan Drive/dev-cache.noindex/cef"
  else
    export CEF_PATH="${XDG_CACHE_HOME:-$HOME/Library/Caches}/nd-cef"
  fi
fi
"""

LINUX_HOOK = """
export CEF_PATH="${CEF_PATH:-${XDG_CACHE_HOME:-$HOME/.cache}/nd-cef}"
# Only the pinned fonts: no host font dirs or /etc/fonts/conf.d, which carry each distro's own
# family preferences.
fonts="${XDG_CACHE_HOME:-$HOME/.cache}/nd-fonts"
mkdir -p "$fonts"
printf '%%s\\n' '<?xml version="1.0"?>' '<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">' '<fontconfig>' \
  "<dir>$FSPEC_PKG_ADWAITA_FONTS</dir>" "<dir>$FSPEC_PKG_NOTO_COLOR_EMOJI</dir>" "<cachedir>$fonts/cache</cachedir>" \
  '<alias><family>sans-serif</family><prefer><family>Adwaita Sans</family></prefer></alias>' \
  '<alias><family>monospace</family><prefer><family>Adwaita Mono</family></prefer></alias>' \
  '</fontconfig>' > "$fonts/fonts.conf"
export FONTCONFIG_FILE="$fonts/fonts.conf"
# Headless tiers render with Mesa's lavapipe when the distro ships it.
for icd in /usr/share/vulkan/icd.d/lvp_icd.*.json; do
  [ -f "$icd" ] && export VK_ICD_FILENAMES="$icd"
done
# Scoped to the host triple: a plain RUSTFLAGS reaches the wasm32 link too, where rust-lld
# rejects -fuse-ld=mold.
export CARGO_TARGET_%s_UNKNOWN_LINUX_GNU_RUSTFLAGS="${CARGO_TARGET_%s_UNKNOWN_LINUX_GNU_RUSTFLAGS:+$CARGO_TARGET_%s_UNKNOWN_LINUX_GNU_RUSTFLAGS }-C link-arg=-fuse-ld=mold"
""" % (platform.arch.upper(), platform.arch.upper(), platform.arch.upper())

dev(
    packages = [
        pkg("rustup", links = {name: "rustup" for name in RUST}),
        pkg("cargo-nextest"),
        pkg("just"),
        pkg("sccache"),
        # nd-terminal's build script compiles libghostty-vt, which needs Zig 0.16.
        pkg("zig", version = "0.16.0"),
        # cef-dll-sys builds CEF's C++ wrapper with CMake and Ninja.
        pkg("cmake"),
        pkg("ninja"),
    ] + ([pkg("mold")] + FONTS if linux else []),
    libs = [lib(n) for n in uniq(GPU + CEF + GST)] if linux else [],
    # labwc and weston are the headless compositors of tier 3; dbus-daemon gives each headless
    # run a private session bus.
    bins = [bin(n) for n in ["git", "rsync", "pkg-config", "clang", "wayland-scanner", "labwc", "weston",
                             "dbus-daemon"]] if linux else [],
    shell_hook = COMMON_HOOK + (LINUX_HOOK if linux else DARWIN_HOOK),
    # Void ships no libsystemd, and moss has no file lookup for the GStreamer plugins: such a
    # host gets the shell in Debian instead.
    fallback_container = "debian:13",
)
