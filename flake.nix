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

      # What libcef.so links on Linux (nixpkgs' cef-binary patches the same list in as its rpath).
      # Web views render with Chromium there, so a dev run of an app with `webview` needs them.
      cefLibs =
        pkgs:
        nixpkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
          with pkgs;
          [
            glib
            nss
            nspr
            atk
            at-spi2-atk
            libdrm
            expat
            libxkbcommon
            libgbm
            gtk3
            pango
            cairo
            alsa-lib
            dbus
            at-spi2-core
            cups
            libGL
            udev
            systemdLibs
            libxcb
            libx11
            libxcomposite
            libxdamage
            libxext
            libxfixes
            libxrandr
            libxshmfence
          ]
        );

      mkDevShell =
        pkgs:
        let
          inherit (pkgs) lib stdenv;
          gpuLibs = runtimeLibs pkgs;
          webLibs = cefLibs pkgs;

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
              # nd-terminal's build script compiles libghostty-vt at the commit libghostty-vt-sys
              # pins, which needs Zig 0.16.
              zig_0_16
              # cef-dll-sys builds CEF's C++ wrapper with CMake and Ninja.
              cmake
              ninja
            ]
            ++ lib.optionals stdenv.hostPlatform.isLinux (
              [
                # libghostty-vt-sys's build script runs `git clone`. The runner's /usr/bin/git
                # loads the nix glibc through LD_LIBRARY_PATH below and dies, so ship one that
                # is linked against the same store.
                git
                pkg-config
                clang
                mold
                wayland-scanner
                mesa
                # Headless compositors for tier 3: labwc lends each test's Hyprland a render node
                # (ndshell::hyprland; Hyprland itself is the system's), weston is the
                # client-decorated case (ndshell::weston).
                labwc
                weston
                # dbus-daemon: each headless run's compositors get a private session bus.
                dbus
                # nd-system's audio dlopens GStreamer; these give its tests playbin, spectrum and wavparse.
                gst_all_1.gstreamer
                gst_all_1.gst-plugins-base
                gst_all_1.gst-plugins-good
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
            # nd-cef's build script downloads the CEF binary distribution (about 500 MB unpacked) into
            # $CEF_PATH; on the macbook that is the cache drive, next to the cargo build dir. Anywhere
            # else (CI runners) it falls back to the user cache.
            if [ -z "''${CEF_PATH:-}" ]; then
              if [ -d "/Volumes/Kyan Drive" ]; then
                export CEF_PATH="/Volumes/Kyan Drive/dev-cache.noindex/cef"
              else
                export CEF_PATH="''${XDG_CACHE_HOME:-$HOME/Library/Caches}/nd-cef"
              fi
            fi
            # Several agents build here at once next to the user's own services; cap each build and
            # test run. Workspace-wide and Linux-capable runs belong on g815 (scripts/linux-gate).
            export CARGO_BUILD_JOBS="''${CARGO_BUILD_JOBS:-4}"
            export NEXTEST_TEST_THREADS="''${NEXTEST_TEST_THREADS:-4}"
          ''
          + lib.optionalString stdenv.hostPlatform.isLinux ''
            export LD_LIBRARY_PATH="${lib.makeLibraryPath (gpuLibs ++ webLibs)}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
            # nd-cef's build script downloads the CEF binary distribution (about 1.3 GB unpacked) into
            # $CEF_PATH, outside the target dir so a `cargo clean` keeps it.
            export CEF_PATH="''${CEF_PATH:-''${XDG_CACHE_HOME:-$HOME/.cache}/nd-cef}"
            export VK_ICD_FILENAMES="${pkgs.mesa}/share/vulkan/icd.d/lvp_icd.${stdenv.hostPlatform.parsed.cpu.name}.json"
            export RUSTFLAGS="''${RUSTFLAGS:+$RUSTFLAGS }-C link-arg=-fuse-ld=mold"
            export LD_LIBRARY_PATH="${lib.makeLibraryPath [ pkgs.gst_all_1.gstreamer ]}:$LD_LIBRARY_PATH"
            export GST_PLUGIN_SYSTEM_PATH_1_0="${
              lib.makeSearchPathOutput "lib" "lib/gstreamer-1.0" (
                with pkgs.gst_all_1;
                [
                  gstreamer
                  gst-plugins-base
                  gst-plugins-good
                ]
              )
            }"
          '';
        };

      mkApp = import ./nix/mk-app.nix {
        inherit
          crane
          mkDevShell
          runtimeLibs
          cefLibs
          ;
      };
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
