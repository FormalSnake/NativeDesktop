const std = @import("std");
const builtin = @import("builtin");

const required_zig = "0.16.0";

pub fn build(b: *std.Build) void {
    checkZigVersion();

    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const gobject = b.dependency("gobject", .{
        .target = target,
        .optimize = optimize,
    });
    const gtk_imports = [_]std.Build.Module.Import{
        .{ .name = "glib", .module = gobject.module("glib2") },
        .{ .name = "gobject", .module = gobject.module("gobject2") },
        .{ .name = "gio", .module = gobject.module("gio2") },
        .{ .name = "gtk", .module = gobject.module("gtk4") },
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
