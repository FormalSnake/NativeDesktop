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
              adwaita-icon-theme # gtk4/libadwaita ship no icon data; without it the headless capture falls back to whatever partial theme the host has
              dbus         # dbus-run-session: the docs capture needs a PRIVATE bus (no settings portal to leak, but real enough for WebKitGTK's dbus-proxy)
              squashfsTools    # M9: AppImage assembly (mksquashfs)
              flatpak-builder  # M9: Flatpak manifest lint (--show-manifest)
              webkitgtk_6_0    # <webview>: dlopen'd libwebkitgtk-6.0.so.4 (src/gtk/webview.zig)
              glib-networking # <webview>: TLS — libsoup loads it as a GIO module; without it every https load fails "TLS support not available"
              gst_all_1.gstreamer        # audio: dlopen'd libgstreamer-1.0.so.0 (src/gtk/audio.zig)
              gst_all_1.gst-plugins-base # audio: playbin
              gst_all_1.gst-plugins-good # audio: spectrum element
            ];
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
            '';
          };
        });
    };
}
