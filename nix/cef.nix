# The Chromium runtime of Linux web views (nd-cef): nixpkgs' cef-binary at the CEF version the
# `cef` crate in crates/nd-cef/Cargo.toml pins (bump both together), laid out as one directory the
# way cef-dll-sys expects it at build time and nd-cef at run time: libcef.so, its resources and
# locales side by side, the C++ wrapper's sources, and the `archive.json` the build checks.
{ pkgs }:
let
  version = "154.0.34";
  gitRevision = "14c5a08";
  chromiumVersion = "154.0.8037.98";
  arch =
    {
      x86_64-linux = "linux64";
      aarch64-linux = "linuxarm64";
    }
    .${pkgs.stdenv.hostPlatform.system};
  binary = pkgs.cef-binary.override {
    inherit version gitRevision chromiumVersion;
    # The SHA-1 digests CEF's build index publishes.
    srcHashes = {
      x86_64-linux = "sha1-Fcka12cJchey6CuouTP+MLE0vmk=";
      aarch64-linux = "sha1-+oV5NRnu8ZnmtvqMyNbw60XG2sI=";
    };
  };
in
pkgs.runCommand "nd-cef-${version}"
  {
    passthru = { inherit version binary; };
  }
  ''
    mkdir -p $out
    cp -r ${binary}/Release/. ${binary}/Resources/. $out/
    cp -r ${binary}/include ${binary}/libcef_dll ${binary}/cmake ${binary}/CMakeLists.txt $out/
    chmod -R u+w $out
    echo '{"type":"minimal","name":"cef_binary_${version}+g${gitRevision}+chromium-${chromiumVersion}_${arch}_minimal.tar.bz2","sha1":""}' > $out/archive.json
  ''
