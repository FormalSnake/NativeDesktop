const std = @import("std");
const builtin = @import("builtin");

const required_zig = "0.16.0";

pub fn build(b: *std.Build) void {
    checkZigVersion();

    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const backend_kind = b.option([]const u8, "backend", "widget backend: gtk|null") orelse "gtk";
    const build_opts = b.addOptions();
    build_opts.addOption([]const u8, "backend", backend_kind);

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
        .{ .name = "build_options", .module = build_opts.createModule() },
    };

    const exe = b.addExecutable(.{
        .name = "nd-hello",
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/main.zig"),
            .target = target,
            .optimize = optimize,
            .imports = &gtk_imports,
        }),
    });
    b.installArtifact(exe);

    const run_step = b.step("run", "Run nd-hello");
    const run_cmd = b.addRunArtifact(exe);
    if (b.args) |args| run_cmd.addArgs(args);
    run_step.dependOn(&run_cmd.step);

    const tests = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/main.zig"),
            .target = target,
            .optimize = optimize,
            .imports = &gtk_imports,
        }),
    });
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
    const style_tests = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/style.zig"),
            .target = target,
            .optimize = optimize,
            .imports = &gtk_imports,
        }),
    });
    test_step.dependOn(&b.addRunArtifact(style_tests).step);

    // `@embedFile` cannot cross a module's package-path boundary (the directory
    // of its root_source_file), so schema/widgets.json — a sibling of src/, not
    // a descendant — can't be embedded directly from src/conformance.zig. Read
    // it here at build.zig time and hand it to the module as a build option.
    const schema_contents = b.build_root.handle.readFileAlloc(b.graph.io, "schema/widgets.json", b.allocator, .limited(1 << 20)) catch @panic("failed to read schema/widgets.json");
    const conformance_opts = b.addOptions();
    conformance_opts.addOption([]const u8, "schema_json", schema_contents);
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

    // Header-conformance test (Task 1): `abi.zig`'s comptime layout asserts
    // run under `zig build test` so header/struct drift fails immediately.
    // No gobject imports — this is the ABI-only test root proving `libnd`'s
    // Zig side compiles without GTK. `build_options` is needed transitively
    // (abi -> tree -> backend); `-lc` is needed for `std.c.environ`
    // (abi.zig's `currentEnviron`, Task 3 — the core reads its own process
    // environment since `nd_init(void)` takes no parameters).
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
