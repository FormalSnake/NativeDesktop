{
  description = "NativeDesktop — Zig + Bun + React native-widget desktop framework";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      forAllSystems = nixpkgs.lib.genAttrs [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
    in
    {
      devShells = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          # The headless weston rig has no session fonts, and this pin's
          # fontconfig cannot parse a newer host /etc/fonts, so `sans` resolved
          # to nothing and GTK fell back to a bitmap face — every capture out of
          # the rig looked wrong. Point fontconfig at our own config naming the
          # GNOME UI font plus a broad fallback.
          # Written by hand rather than via makeFontsConf: that helper also
          # pulls in the user's own font directories, so `sans` resolved to
          # whatever happened to be installed and captures were not
          # reproducible between machines.
          # libcef.so is a generic-Linux binary with 26 sonames NixOS does not
          # put anywhere a plain dlopen can see. They stay OUT of the shell's
          # own LD_LIBRARY_PATH (the webkitgtk path above is deliberate about
          # what is on it) and are exported as ND_CEF_LD_LIBRARY_PATH for the
          # CEF gate to prepend when it launches the host.
          cefRuntimeLibs = pkgs.lib.optionals pkgs.stdenv.isLinux (with pkgs; [
            nss nspr cups dbus expat alsa-lib libdrm libgbm systemd
            at-spi2-core at-spi2-atk atk libxkbcommon libGL
            glib cairo pango
            libx11 libxcomposite libxdamage libxext libxfixes libxrandr
            libxcb libxrender libxi libxtst libxcursor
          ]);
          fontsConf = pkgs.writeText "nd-headless-fonts.conf" ''
            <?xml version="1.0"?>
            <!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
            <fontconfig>
              <dir>${pkgs.adwaita-fonts}/share/fonts</dir>
              <dir>${pkgs.dejavu_fonts}/share/fonts</dir>
              <dir>${pkgs.noto-fonts-color-emoji}/share/fonts</dir>
              <cachedir prefix="xdg">fontconfig</cachedir>
              <alias><family>sans-serif</family><prefer>
                <family>Adwaita Sans</family><family>DejaVu Sans</family>
              </prefer></alias>
              <alias><family>sans</family><prefer>
                <family>Adwaita Sans</family><family>DejaVu Sans</family>
              </prefer></alias>
              <alias><family>serif</family><prefer><family>DejaVu Serif</family></prefer></alias>
              <alias><family>monospace</family><prefer>
                <family>Adwaita Mono</family><family>DejaVu Sans Mono</family>
              </prefer></alias>
              <alias><family>emoji</family><prefer><family>Noto Color Emoji</family></prefer></alias>
              <!-- libadwaita on this pin still asks for "Cantarell" by name.
                   Unaliased, fontconfig answers whatever sorts first, which is
                   the MONO face, and every label in a capture comes out
                   monospaced. -->
              <alias><family>Cantarell</family><prefer><family>Adwaita Sans</family></prefer></alias>
              <alias><family>Inter</family><prefer><family>Adwaita Sans</family></prefer></alias>
              <!-- Last resort for any family none of the above names: sans, not
                   whatever the sort order happens to put first. -->
              <match target="pattern">
                <edit name="family" mode="append_last"><string>Adwaita Sans</string></edit>
              </match>
              <match target="font"><edit name="antialias" mode="assign"><bool>true</bool></edit></match>
              <match target="font"><edit name="hinting" mode="assign"><bool>true</bool></edit></match>
              <match target="font"><edit name="hintstyle" mode="assign"><const>hintslight</const></edit></match>
              <match target="font"><edit name="rgba" mode="assign"><const>none</const></edit></match>
            </fontconfig>
          '';
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              zig
              zls
              bun
              pkg-config
              minisign     # M9: sign/verify update manifests + archives
              zstd         # M9: .tar.zst full-archive updates (Linux)
            ] ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
              gtk4
              libadwaita
              glib
              gobject-introspection
              libxslt      # provides xsltproc for zig-gobject codegen
              weston       # headless wayland compositor for CI/agent runs
              xwayland     # `weston --xwayland`: CEF's windowed embedding is X11-only, so the M2 webview drive needs an X server inside the weston rig
              adwaita-icon-theme # gtk4/libadwaita ship no icon data; without it the headless capture falls back to whatever partial theme the host has
              fontconfig     # fc-match/fc-list, and the library the rig's FONTCONFIG_FILE is written for
              adwaita-fonts  # Adwaita Sans/Mono — GNOME 48's UI fonts, so a headless capture looks like a real session
              dejavu_fonts   # broad fallback for anything Adwaita Sans lacks
              dbus         # dbus-run-session: the docs capture needs a PRIVATE bus (no settings portal to leak, but real enough for WebKitGTK's dbus-proxy)
              squashfsTools    # M9: AppImage assembly (mksquashfs)
              flatpak-builder  # M9: Flatpak manifest lint (--show-manifest)
              webkitgtk_6_0    # <webview>: dlopen'd libwebkitgtk-6.0.so.4 (src/gtk/webview.zig)
              glib-networking # <webview>: TLS — libsoup loads it as a GIO module; without it every https load fails "TLS support not available"
              gst_all_1.gstreamer        # audio: dlopen'd libgstreamer-1.0.so.0 (src/gtk/audio.zig)
              gst_all_1.gst-plugins-base # audio: playbin
              gst_all_1.gst-plugins-good # audio: spectrum element
              gtksourceview5   # <codeeditor>: dlopen'd libgtksourceview-5.so.0 (src/gtk/codeeditor.zig)
              xorg-server      # Xvfb: the CEF gate needs a real X11 root window (windowed embedding is X11-only, and an XWayland root paints nothing to screenshot)
              xwininfo         # the no-stray-window census: `xwininfo -root -children` before and after a popup
              imagemagick      # `import -window root`: the only capture that includes the X11 child window CEF renders into
            ] ++ cefRuntimeLibs;
            # build.zig's test roots import the gobject binding modules
            # unconditionally, so `zig build test` needs pkg-config to resolve
            # gtk4/libadwaita even on the Mac (compile/test-only there — the
            # shipping Mac backend is AppKit). nixpkgs' libadwaita does not
            # build on darwin (its appstream dep fails), so the darwin shell
            # borrows Homebrew's GTK stack: `brew install libadwaita`.
            shellHook = pkgs.lib.optionalString pkgs.stdenv.isDarwin ''
              if [ -d /opt/homebrew/lib/pkgconfig ]; then
                export PKG_CONFIG_PATH="/opt/homebrew/lib/pkgconfig:/opt/homebrew/share/pkgconfig''${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
              fi
            '' + pkgs.lib.optionalString pkgs.stdenv.isLinux ''
              # webview + audio are dlopen'd by bare soname (never linked), so the
              # host only finds them if their lib dirs are on the runtime search
              # path — and they must come from THIS nixpkgs pin: a system-nixpkgs
              # webkitgtk dedupes against the dev shell's already-loaded glib/gst
              # sonames and fails to relocate (undefined symbol at dlopen).
              export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.webkitgtk_6_0 pkgs.gst_all_1.gstreamer pkgs.gst_all_1.gst-plugins-base ]}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
              export GST_PLUGIN_SYSTEM_PATH_1_0="${pkgs.lib.makeSearchPath "lib/gstreamer-1.0" [ pkgs.gst_all_1.gstreamer pkgs.gst_all_1.gst-plugins-base pkgs.gst_all_1.gst-plugins-good ]}"
              export GIO_EXTRA_MODULES="${pkgs.glib-networking}/lib/gio/modules''${GIO_EXTRA_MODULES:+:$GIO_EXTRA_MODULES}"
              # Never inherit the host's /etc/fonts: it is written for whatever
              # fontconfig the OS ships, and this pin's parser rejects its alias
              # rules ("invalid constant used : sans-serif"), leaving `sans`
              # unresolvable.
              export FONTCONFIG_FILE="${fontsConf}"
              export ND_CEF_LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath cefRuntimeLibs}"
            '';
          };
        });
    };
}
