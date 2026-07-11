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
          };
        });
    };
}
