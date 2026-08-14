#!/usr/bin/env bash
# Builds the AppKit host: GTK-free libnd.a, repacked for Apple's ld, then the
# SwiftPM shell. Shared by release.yml, mac.yml, and package.yml; run it with
# `env -u SDKROOT` when a Nix devshell has SDKROOT set.
# Prints the artifact path as its last line.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT="$(pwd)"

zig build libnd -Dbackend=abi

# Zig's archiver emits members Apple's ld rejects ("not 8-byte aligned") and
# extracts them 0-permission; repack with the system ar/libtool before linking.
workdir="$(mktemp -d)"
( cd "$workdir" && ar x "$ROOT/zig-out/lib/libnd.a" && chmod 644 *.o && libtool -static -o "$ROOT/zig-out/lib/libnd.a" *.o )
rm -rf "$workdir"

# libnd.a reaches the link through .unsafeFlags (swift/Package.swift), which
# llbuild does not track as an input, so swift build will happily keep an
# executable built against an older archive when only Zig changed. That ships a
# stale host from a green build, and the backtrace still resolves against the
# fresh DWARF on disk, so it lies about which code ran. Force the relink.
rm -f "$ROOT/swift/.build/arm64-apple-macosx/release/NDShell" "$ROOT/swift/.build/release/NDShell"
( cd swift && swift build -c release )
echo "$ROOT/swift/.build/release/NDShell"
