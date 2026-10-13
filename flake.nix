{
  description = "NativeDesktop dev shell";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.crane.url = "github:ipetkov/crane";

  outputs =
    { nixpkgs, crane, ... }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      # Libraries GPUI loads with dlopen on Linux.
      runtimeLibs =
        pkgs:
        nixpkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
          with pkgs;
          [
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
          ]
        );

      mkDevShell =
        pkgs:
        let
          inherit (pkgs) lib stdenv;
          gpuLibs = runtimeLibs pkgs;

          # A darwin stdenv carries the apple-sdk setup hook, which exports SDKROOT and
          # DEVELOPER_DIR and pins Swift and the linker to nixpkgs' old SDK.
          mkShell = if stdenv.hostPlatform.isDarwin then pkgs.mkShellNoCC else pkgs.mkShell;
        in
        mkShell {
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
                # Headless compositors for tier 3: labwc lends each test's Hyprland a render node
                # (ndshell::hyprland; Hyprland itself is the system's), weston is the
                # client-decorated case (ndshell::weston).
                labwc
                weston
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

      mkApp = import ./nix/mk-app.nix { inherit crane mkDevShell runtimeLibs; };
      hmModule = import ./nix/hm-module.nix;
      hmEvalCheck = import ./nix/hm-eval-check.nix { lib = nixpkgs.lib; };

      # The settings schema of examples/notes, committed as a fixture of nd-settings' tests.
      notesFixture = ./crates/nd-settings/tests/fixtures/notes;
      notesPluginsFixture = ./crates/nd-settings/tests/fixtures/notes-plugins;
    in
    {
      devShells = forAll (pkgs: {
        default = mkDevShell pkgs;
      });

      lib = {
        inherit mkApp;
        # `{ pname, settingsOptions, defaultPackage }`, for apps that build their package themselves.
        mkHomeModule = hmModule;
      };

      checks = forAll (pkgs: {
        hm-module-eval-notes-plugins = hmEvalCheck {
          inherit pkgs;
          pname = "notes";
          homeModule = hmModule {
            pname = "notes";
            settingsOptions = import (notesPluginsFixture + "/nix/settings-options.nix");
            defaultPackage = _: pkgs.hello;
          };
          settings = {
            compact_rows = true;
            plugins.discord.enabled = false;
            plugins.other.anything = "passes";
          };
          expectLines = [
            "compact_rows = true"
            "enabled = false"
          ];
          rejects = [
            { plugins.discord.enabled = "no"; }
            { plugins.discord.buttons = 3; }
          ];
        };

        hm-module-eval-notes = hmEvalCheck {
          inherit pkgs;
          pname = "notes";
          homeModule = hmModule {
            pname = "notes";
            settingsOptions = import (notesFixture + "/nix/settings-options.nix");
            defaultPackage = _: pkgs.hello;
          };
          settings = {
            compact_rows = true;
            editor_font_size = 18;
            new_note_folder = "pinned";
            plugins.discord.application_id = "123";
          };
          expectLines = [
            "compact_rows = true"
            "editor_font_size = 18"
            "new_note_folder = \"pinned\""
          ];
          absent = [ "spellcheck" ];
          rejects = [
            { editor_font_size = 99; }
            { editor_font_size = "big"; }
            { new_note_folder = "elsewhere"; }
            { new_note_flder = "pinned"; }
          ];
        };
      });
    };
}
