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
              squashfsTools    # M9: AppImage assembly (mksquashfs)
              flatpak-builder  # M9: Flatpak manifest lint (--show-manifest)
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
            '';
          };
        });
    };
}
