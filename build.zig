const std = @import("std");
const builtin = @import("builtin");

const required_zig = "0.16.0";

pub fn build(b: *std.Build) void {
    checkZigVersion();

    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    // The comptime seam is `null` (in-process conformance backend)
    // or `abi` (the C-ABI vtable seam every real embedder — GTK included —
    // routes through). There is no bare `gtk` seam value: GTK-ness lives
    // in the embedder (src/gtk/), not the seam.
    const backend_kind = b.option([]const u8, "backend", "widget backend: abi|null") orelse "abi";
    const build_opts = b.addOptions();
    build_opts.addOption([]const u8, "backend", backend_kind);

    // The CEF <webview> engine compiles in only when a CEF binary distribution
    // is on hand to translate `include/capi/*.h` against. Nothing from the dist
    // is linked (libcef.so is dlopen'd at runtime, see src/cef/loader.zig);
    // this is headers only, and the pin below must match `cef.api_version`.
    const cef_dist = resolveCefDist(b, target);
    build_opts.addOption(bool, "cef_engine", cef_dist != null);
    const build_options_mod = build_opts.createModule();

    const gobject = b.dependency("gobject", .{
        .target = target,
        .optimize = optimize,
    });
    const gtk_imports = [_]std.Build.Module.Import{
        .{ .name = "glib", .module = gobject.module("glib2") },
        .{ .name = "gobject", .module = gobject.module("gobject2") },
        .{ .name = "gio", .module = gobject.module("gio2") },
        .{ .name = "gtk", .module = gobject.module("gtk4") },
        .{ .name = "gsk", .module = gobject.module("gsk4") },
        .{ .name = "gdk", .module = gobject.module("gdk4") },
        .{ .name = "graphene", .module = gobject.module("graphene1") },
        .{ .name = "adw", .module = gobject.module("adw1") },
        .{ .name = "cairo", .module = gobject.module("cairo1") },
        .{ .name = "pango", .module = gobject.module("pango1") },
        .{ .name = "pangocairo", .module = gobject.module("pangocairo1") },
        .{ .name = "build_options", .module = build_options_mod },
    };

    // Prebuilt libghostty-vt static archive. Linked into every artifact
    // that compiles the generated GTK create dispatcher — which reaches
    // src/gtk/terminal.zig -> src/core/terminal.zig and its ghostty_* externs.
    // Per-arch blobs live side by side so a Linux and a macOS checkout never
    // clobber each other's archive on sync; the Swift/AppKit build picks the
    // macOS one directly in swift/Package.swift.
    const ghostty_vt_lib = b.path(switch (target.result.os.tag) {
        .macos => "vendor/libghostty-vt/lib/libghostty-vt-macos-aarch64.a",
        .linux => "vendor/libghostty-vt/lib/libghostty-vt-linux-x86_64.a",
        else => @panic("no prebuilt libghostty-vt for this target"),
    });

    // ---- The core: `src/abi.zig` transitively reaches every other
    // GTK-free core file via ordinary same-directory relative imports (abi
    // -> {abi_backend, tree, runtime, automation, protocol}; tree/runtime/
    // automation -> backend.zig -> {null_backend, abi_backend}) — so it is
    // the one module root that covers the whole core surface. No gobject
    // imports anywhere in this module; `-lc` is needed for `std.c.environ`
    // (abi.zig's `currentEnviron`).
    const abi_mod = b.createModule(.{
        .root_source_file = b.path("src/abi.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
        .imports = &.{.{ .name = "build_options", .module = build_options_mod }},
    });

    // `src/core/root.zig` is `libnd`'s module root — it lives in a
    // subdirectory, so it reaches `abi.zig` (which stays flat under `src/`)
    // via a named import rather than a relative `@import("../abi.zig")`
    // (Zig 0.16 forbids a module's `@import` escaping its root directory).
    const core_mod = b.createModule(.{
        .root_source_file = b.path("src/core/root.zig"),
        .target = target,
        .optimize = optimize,
        .imports = &.{.{ .name = "abi", .module = abi_mod }},
    });

    // ---- Linux exe: rooted at `src/nd_hello_root.zig`, not `src/gtk/main.zig`
    // directly (see that file's doc comment) — `src/gtk/*.zig` reach
    // `abi.zig`/`tree.zig`/etc. via `../` relative imports, which needs the
    // module's root directory to be `src/` (an ancestor of `src/gtk/`),
    // achieved only by the shim — Zig 0.16 forbids a module's `@import`
    // escaping ITS root directory, and rooting directly at
    // `src/gtk/main.zig` makes `src/gtk/` that boundary instead.
    //
    // `nd_hello_root.zig` also re-exports `generated/widgets.zig`'s public
    // surface at its own top level, and its module self-aliases under the
    // name "generated" (`addImport("generated", <itself>)`) — `style.zig`/
    // `backend.zig` use `@import("generated")` uniformly (they must resolve
    // to ONE shared type identity, and Zig 0.16 forbids two separately-
    // constructed modules from both reaching the same file, so a second,
    // independently-built `generated_shim`-rooted module cannot coexist
    // here with the exe's own `src/`-rooted tree, which already reaches
    // `src/protocol.zig` directly).
    const exe_mod = b.createModule(.{
        .root_source_file = b.path("src/nd_hello_root.zig"),
        .target = target,
        .optimize = optimize,
        .imports = &gtk_imports,
    });
    exe_mod.addImport("generated", exe_mod);
    linkTerminalDeps(exe_mod, ghostty_vt_lib, target);
    addCefHeaders(exe_mod, cef_dist);
    const exe = b.addExecutable(.{ .name = "nd-hello", .root_module = exe_mod });
    b.installArtifact(exe);

    const run_step = b.step("run", "Run nd-hello");
    const run_cmd = b.addRunArtifact(exe);
    if (b.args) |args| run_cmd.addArgs(args);
    run_step.dependOn(&run_cmd.step);

    const tests_mod = b.createModule(.{
        .root_source_file = b.path("src/nd_hello_root.zig"),
        .target = target,
        .optimize = optimize,
        .imports = &gtk_imports,
    });
    tests_mod.addImport("generated", tests_mod);
    linkTerminalDeps(tests_mod, ghostty_vt_lib, target);
    addCefHeaders(tests_mod, cef_dist);
    const tests = b.addTest(.{ .root_module = tests_mod });
    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&b.addRunArtifact(tests).step);

    const protocol_tests = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/protocol.zig"),
            .target = target,
            .optimize = optimize,
        }),
    });
    test_step.dependOn(&b.addRunArtifact(protocol_tests).step);

    const tree_tests = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/tree.zig"),
            .target = target,
            .optimize = optimize,
            .imports = &gtk_imports,
        }),
    });
    test_step.dependOn(&b.addRunArtifact(tree_tests).step);

    // `test` declarations are only collected from a file's own addTest root,
    // never transitively through @import (Zig 0.16) — style.zig's compileCss
    // unit tests need their own root, or `zig build test` silently skips them.
    // Its test module's root directory is `src/gtk/` (root_source_file =
    // src/gtk/style.zig), which cannot reach `src/generated/widgets.zig` via
    // `../` (outside that root — Zig 0.16 forbids a module's `@import`
    // escaping its own root directory). This is a wholly separate artifact
    // from the exe/tests above, so a dedicated `generated_shim`-rooted
    // module (reaching `src/protocol.zig` on its own) causes no cross-
    // module file-ownership conflict here — unlike inside the exe's own
    // artifact, where the same trick would collide with the exe's direct
    // access to `src/protocol.zig` (hence that context uses a self-alias
    // instead, see `exe_mod`/`tests_mod` above).
    const style_test_generated_mod = b.createModule(.{
        .root_source_file = b.path("src/generated_shim.zig"),
        .target = target,
        .optimize = optimize,
        .imports = &gtk_imports,
    });
    linkTerminalDeps(style_test_generated_mod, ghostty_vt_lib, target);
    const style_tests_mod = b.createModule(.{
        .root_source_file = b.path("src/gtk/style.zig"),
        .target = target,
        .optimize = optimize,
        .imports = &(gtk_imports ++ [_]std.Build.Module.Import{
            .{ .name = "generated", .module = style_test_generated_mod },
        }),
    });
    linkTerminalDeps(style_tests_mod, ghostty_vt_lib, target);
    const style_tests = b.addTest(.{ .root_module = style_tests_mod });
    test_step.dependOn(&b.addRunArtifact(style_tests).step);

    // `@embedFile` cannot cross a module's package-path boundary (the directory
    // of its root_source_file), so schema/widgets.json — a sibling of src/, not
    // a descendant — can't be embedded directly from src/conformance.zig. Read
    // it here at build.zig time and hand it to the module as a build option.
    const schema_contents = b.build_root.handle.readFileAlloc(b.graph.io, "schema/widgets.json", b.allocator, .limited(1 << 20)) catch @panic("failed to read schema/widgets.json");
    const conformance_opts = b.addOptions();
    conformance_opts.addOption([]const u8, "schema_json", schema_contents);
    // `tree.zig`'s `apply()` (exercised by the update-op conformance test) reaches
    // `backend.zig`, which reads `build_options.backend` to pick null vs. abi —
    // force "null" here so the conformance suite stays display-free (no gobject).
    conformance_opts.addOption([]const u8, "backend", "null");
    const conformance_tests = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/conformance.zig"),
            .target = target,
            .optimize = optimize,
            .imports = &.{
                .{ .name = "build_options", .module = conformance_opts.createModule() },
            },
        }),
    });
    test_step.dependOn(&b.addRunArtifact(conformance_tests).step);

    // Header-conformance test: `abi.zig`'s comptime layout asserts
    // run under `zig build test` so header/struct drift fails immediately.
    // No gobject imports — this is the ABI-only test root proving `libnd`'s
    // Zig side compiles without GTK.
    const abi_tests = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/abi.zig"),
            .target = target,
            .optimize = optimize,
            .link_libc = true,
            .imports = &.{
                .{ .name = "build_options", .module = build_opts.createModule() },
            },
        }),
    });
    test_step.dependOn(&b.addRunArtifact(abi_tests).step);

    // Update-verification core: its own addTest root — Zig 0.16 does not
    // collect `test {}` blocks transitively through @import, so without this
    // update.zig's minisign tests are silently skipped.
    const update_tests = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/core/update.zig"),
            .target = target,
            .optimize = optimize,
        }),
    });
    test_step.dependOn(&b.addRunArtifact(update_tests).step);

    // Binary NDP decoder: own addTest root (transitive test collection
    // through @import does not happen in Zig 0.16).
    const ndp_binary_tests = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/ndp_binary.zig"),
            .target = target,
            .optimize = optimize,
        }),
    });
    test_step.dependOn(&b.addRunArtifact(ndp_binary_tests).step);

    // Terminal core virtual-mode tests: own addTest root (Zig 0.16 does not
    // collect `test {}` transitively) — feed/reset/input-routing/title against a
    // real libghostty-vt, no PTY. Links the terminal deps (ghostty + libc).
    const terminal_tests_mod = b.createModule(.{
        .root_source_file = b.path("src/core/terminal.zig"),
        .target = target,
        .optimize = optimize,
    });
    linkTerminalDeps(terminal_tests_mod, ghostty_vt_lib, target);
    const terminal_tests = b.addTest(.{ .root_module = terminal_tests_mod });
    test_step.dependOn(&b.addRunArtifact(terminal_tests).step);

    // Remote-terminal transport: own addTest root (Zig 0.16 does not collect
    // `test {}` transitively) for the standalone byte-plane framing tests. It
    // imports terminal.zig (ghostty_* externs), so link the terminal deps.
    const remote_terminal_tests_mod = b.createModule(.{
        .root_source_file = b.path("src/core/remote_terminal.zig"),
        .target = target,
        .optimize = optimize,
    });
    linkTerminalDeps(remote_terminal_tests_mod, ghostty_vt_lib, target);
    const remote_terminal_tests = b.addTest(.{ .root_module = remote_terminal_tests_mod });
    test_step.dependOn(&b.addRunArtifact(remote_terminal_tests).step);

    // Capability ACL: own addTest root, std-only module.
    const acl_tests = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/acl.zig"),
            .target = target,
            .optimize = optimize,
        }),
    });
    test_step.dependOn(&b.addRunArtifact(acl_tests).step);

    // Automation dialog script (FIFO/exhausted/unscripted): own addTest root;
    // link_libc for the std.c.getenv/fopen env+file reads.
    const dialog_script_tests = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/automation_dialogs.zig"),
            .target = target,
            .optimize = optimize,
            .link_libc = true,
        }),
    });
    test_step.dependOn(&b.addRunArtifact(dialog_script_tests).step);

    // Automation server (waitFor vocabulary / value rendering): own addTest
    // root. backend "null" keeps the tree->backend seam display-free.
    const automation_opts = b.addOptions();
    automation_opts.addOption([]const u8, "backend", "null");
    const automation_tests = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/automation.zig"),
            .target = target,
            .optimize = optimize,
            .link_libc = true,
            .imports = &.{
                .{ .name = "build_options", .module = automation_opts.createModule() },
            },
        }),
    });
    test_step.dependOn(&b.addRunArtifact(automation_tests).step);

    // Native-plugin dlopen loader: own addTest root; link_libc because
    // the test dispatches into the demo plugin's libc-allocated result and
    // frees it with std.c.free.
    const plugin_tests = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/plugin.zig"),
            .target = target,
            .optimize = optimize,
            .link_libc = true,
        }),
    });
    test_step.dependOn(&b.addRunArtifact(plugin_tests).step);

    // `nd-update-verify`: bytes-in → exit 0/1 CLI wrapping verifyMinisign,
    // used by scripts/m9-drive.ts to run the non-disableable signature check.
    const update_verify_mod = b.createModule(.{
        .root_source_file = b.path("src/core/update_verify_main.zig"),
        .target = target,
        .optimize = optimize,
    });
    const update_verify_exe = b.addExecutable(.{ .name = "nd-update-verify", .root_module = update_verify_mod });
    const update_verify_step = b.step("update-verify", "Build nd-update-verify (minisign check CLI)");
    update_verify_step.dependOn(&b.addInstallArtifact(update_verify_exe, .{}).step);
    b.installArtifact(update_verify_exe);

    // ---- libnd.a: the static lib for the Mac, rooted at the same
    // GTK-free core, `-Dbackend=abi`, no gobject imports at all.
    const libnd = b.addLibrary(.{
        .name = "nd",
        .linkage = .static,
        .root_module = core_mod,
    });
    // Bundle Zig's compiler-rt (f128 soft-float builtins: __divtf3 etc., pulled
    // in by std.fmt.parse_float / std.json) into the archive itself. Without
    // this, `libnd.a` links fine with `zig build`'s own driver (which supplies
    // compiler-rt implicitly) but fails with undefined symbols under any other
    // linker driver (e.g. swiftc/SwiftPM) that doesn't know about Zig's runtime
    // support code. Self-contained archive, no downstream linker flags needed.
    libnd.bundle_compiler_rt = true;
    const libnd_step = b.step("libnd", "Build libnd.a (static, GTK-free, -Dbackend=abi)");
    const libnd_install = b.addInstallArtifact(libnd, .{});
    libnd_step.dependOn(&libnd_install.step);
    // Zig 0.16 emits archive members the new Apple ld rejects ("libnd_zcu.o
    // is not 8-byte aligned"); repack the installed archive with libtool so
    // swiftc/SwiftPM can link it. Host-gated: only Apple's linker cares, and
    // libtool only exists on macOS. Repacking is idempotent.
    //
    // libtool must be handed the extracted OBJECT files, not the archive:
    // `libtool -static -o out in.a` on macOS SILENTLY DROPS whichever member is
    // misaligned (keeping only the aligned one), so repacking straight from the
    // archive loses libnd_zcu.o once it is the misaligned member. Extract, then
    // repack from the .o files, which libtool aligns correctly.
    if (builtin.os.tag == .macos) {
        const lib_path = b.getInstallPath(.lib, "libnd.a");
        const repack = b.addSystemCommand(&.{
            "/bin/sh", "-c",
            b.fmt(
                "set -e; d=$(dirname '{s}'); w=\"$d/.ndrepack\"; rm -rf \"$w\"; mkdir \"$w\"; (cd \"$w\" && ar x '{s}' && chmod +r *.o && libtool -static -o repacked.a *.o); mv \"$w/repacked.a\" '{s}'; rm -rf \"$w\"",
                .{ lib_path, lib_path, lib_path },
            ),
        });
        repack.step.dependOn(&libnd_install.step);
        libnd_step.dependOn(&repack.step);
    }
    libnd_step.dependOn(&b.addInstallFileWithDir(b.path("include/nd.h"), .{ .custom = "include" }, "nd.h").step);
    // nd.h nests `#include "nd_plugin.h"` — install it alongside or
    // the installed header tree fatally fails to resolve for any consumer.
    libnd_step.dependOn(&b.addInstallFileWithDir(b.path("include/nd_plugin.h"), .{ .custom = "include" }, "nd_plugin.h").step);

    // The published @nativedesktop/native package ships a copy of the two ABI
    // headers; ci cmp-checks it against include/. Not in the default graph —
    // run after editing include/.
    const sync_headers_step = b.step("sync-native-headers", "Copy include/nd*.h into packages/native/include/");
    sync_headers_step.dependOn(&b.addSystemCommand(&.{ "scripts/sync-native-headers.sh" }).step);

    // First-party demo plugin: a C-ABI shared lib exporting
    // nd_plugin_entry. Built as its own artifact; headless-m10.sh dlopens it.
    const plugin_hello = b.addLibrary(.{
        .name = "nd_plugin_hello",
        .linkage = .dynamic,
        .root_module = b.createModule(.{
            .root_source_file = b.path("plugins/hello/plugin.zig"),
            .target = target,
            .optimize = optimize,
            .link_libc = true,
        }),
    });
    const plugin_step = b.step("plugin-hello", "Build the hello demo plugin (.so)");
    plugin_step.dependOn(&b.addInstallArtifact(plugin_hello, .{}).step);

    // Demo native-view module: a C-ABI shared lib exporting
    // nd_plugin_entry that registers its OWN GtkWidget under the "colorview"
    // view kind via the v2 plugin ABI (register_view). Links the GTK imports
    // (their pkg-config system libs propagate to the .dylib; GtkWidget/cairo
    // resolve against the same libgtk the host already loaded). Proves a
    // third-party module can host a native view with zero core schema edits.
    const plugin_colorview = b.addLibrary(.{
        .name = "nd_plugin_colorview",
        .linkage = .dynamic,
        .root_module = b.createModule(.{
            .root_source_file = b.path("plugins/colorview/plugin.zig"),
            .target = target,
            .optimize = optimize,
            .link_libc = true,
            .imports = &gtk_imports,
        }),
    });
    const plugin_colorview_step = b.step("plugin-colorview", "Build the colorview demo native-view module (.so)");
    plugin_colorview_step.dependOn(&b.addInstallArtifact(plugin_colorview, .{}).step);
}

// Link the terminal core's native deps into any artifact that compiles
// src/core/terminal.zig: the prebuilt libghostty-vt archive, libc (forkpty +
// threads), and libutil on Linux (forkpty lives there; on macOS it's libSystem).
/// Root of a CEF binary distribution, the directory holding `include/`,
/// `Release/` and `Resources/`. `-Dcef-dist=` wins, then `ND_CEF_ROOT`, then
/// the dev cache the packaging lane also writes to. Linux targets only: the
/// windowed embedding this engine does is X11-only.
fn resolveCefDist(b: *std.Build, target: std.Build.ResolvedTarget) ?[]const u8 {
    const explicit = b.option([]const u8, "cef-dist", "root of a CEF binary distribution (headers only; libcef.so is dlopen'd)");
    if (target.result.os.tag != .linux) return null;
    const root = explicit orelse
        b.graph.environ_map.get("ND_CEF_ROOT") orelse
        b.pathJoin(&.{ b.graph.environ_map.get("HOME") orelse return null, ".cache/nativedesktop/cef/" ++ cef_version_dir });
    std.Io.Dir.accessAbsolute(b.graph.io, b.pathJoin(&.{ root, "include", "capi", "cef_app_capi.h" }), .{}) catch {
        if (explicit != null) std.debug.panic("-Dcef-dist={s} is not a CEF distribution (no include/capi/cef_app_capi.h)", .{root});
        return null;
    };
    return root;
}

/// The dist's own root is the include path: the capi headers include each
/// other as `include/capi/...`, relative to the distribution root.
fn addCefHeaders(mod: *std.Build.Module, dist: ?[]const u8) void {
    const root = dist orelse return;
    mod.addIncludePath(.{ .cwd_relative = root });
}

const cef_version_dir = "151.3.23-linux64";

fn linkTerminalDeps(mod: *std.Build.Module, ghostty_lib: std.Build.LazyPath, target: std.Build.ResolvedTarget) void {
    mod.link_libc = true;
    mod.addObjectFile(ghostty_lib);
    if (target.result.os.tag == .linux) mod.linkSystemLibrary("util", .{});
}

fn checkZigVersion() void {
    const required = std.SemanticVersion.parse(required_zig) catch unreachable;
    if (builtin.zig_version.order(required) != .eq) {
        std.debug.panic(
            "NativeDesktop requires Zig {s} exactly (found {s}). Enter the devshell (direnv/nix develop).",
            .{ required_zig, builtin.zig_version_string },
        );
    }
}
