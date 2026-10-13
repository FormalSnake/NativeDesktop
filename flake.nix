{
  description = "NativeDesktop dev shell";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { nixpkgs, ... }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      devShells = forAll (
        pkgs:
        let
          inherit (pkgs) lib stdenv;

          # Libraries GPUI loads with dlopen on Linux.
          gpuLibs = with pkgs; [
            wayland
            libxkbcommon
            vulkan-loader
            libx11
            libxcb
            libxcursor
            libxi
            libxrandr
            libxkbfile
            fontconfig
            freetype
            alsa-lib
            openssl
            libgbm
            libglvnd
          ];

          # A darwin stdenv carries the apple-sdk setup hook, which exports SDKROOT and
          # DEVELOPER_DIR and pins Swift and the linker to nixpkgs' old SDK.
          mkShell = if stdenv.hostPlatform.isDarwin then pkgs.mkShellNoCC else pkgs.mkShell;
        in
        {
          default = mkShell {
            packages =
              with pkgs;
              [
                rustup
                cargo-nextest
                just
                sccache
              ]
              ++ lib.optionals stdenv.hostPlatform.isLinux (
                [
                  pkg-config
                  clang
                  mold
                  cmake
                  wayland-scanner
                  mesa
                  # Headless parent compositor that lends each test's Hyprland a render node
                  # (ndshell::hyprland). Hyprland itself is the system's.
                  labwc
                ]
                ++ gpuLibs
              );

            shellHook = ''
              # rust-toolchain.toml is resolved by the rustup proxies. A nix rustc or cargo-clippy ahead
              # of them bypasses the pin, and mixing compilers in one build dir fails with E0514.
              export PATH="''${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
              unset RUSTC CARGO RUSTDOC
            ''
            + lib.optionalString stdenv.hostPlatform.isDarwin ''
              unset SDKROOT DEVELOPER_DIR MACOSX_DEPLOYMENT_TARGET NIX_CFLAGS_COMPILE NIX_LDFLAGS
              # The system toolchain in /usr/bin must win. nixpkgs' xcrun cannot find the macosx SDK, and a
              # nix strip (rustc's release strip step) corrupts proc-macro dylibs: "mis-aligned LINKEDIT".
              # This also covers a nix cc, cctools or rustc inherited from the calling environment.
              PATH=$(printf %s "$PATH" | tr ':' '\n' | grep -v -e '-xcbuild-' -e '-xcrun' -e '-cctools' -e '-clang' -e '-binutils' -e '-rustc-wrapper' -e '-cargo-1' | paste -sd: -)
              export PATH="/usr/bin:$PATH"
              for v in $(env | sed -n 's/^\(NIX_\(CC\|BINTOOLS\|CFLAGS\|LDFLAGS\|HARDENING\|APPLE\)[A-Za-z0-9_]*\)=.*/\1/p'); do unset "$v"; done
            ''
            + lib.optionalString stdenv.hostPlatform.isLinux ''
              export LD_LIBRARY_PATH="${lib.makeLibraryPath gpuLibs}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
              export VK_ICD_FILENAMES="${pkgs.mesa}/share/vulkan/icd.d/lvp_icd.${stdenv.hostPlatform.parsed.cpu.name}.json"
              export RUSTFLAGS="''${RUSTFLAGS:+$RUSTFLAGS }-C link-arg=-fuse-ld=mold"
            '';
          };
        }
      );
    };
}
