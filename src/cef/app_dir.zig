//! The name the Chromium profile root is keyed on, read the way getAppDataDir()
//! (packages/react/src/paths.ts) reads it: the bundle's nd-app.json `dataName`
//! (or `name`, for bundles packaged before dataName existed), else the cwd's
//! package.json `name`. The root is `<data dir>/<name>/cef`, so dev and packaged
//! runs of one app share it and two apps never do. NDCefEngine.swift does the
//! same on macOS.

const std = @import("std");

pub const manifest_keys = [_][]const u8{ "dataName", "name" };
pub const package_keys = [_][]const u8{"name"};

/// The first non-empty string among `keys` in a JSON object, duplicated.
pub fn nameFrom(alloc: std.mem.Allocator, json: []const u8, keys: []const []const u8) ?[]u8 {
    const parsed = std.json.parseFromSlice(std.json.Value, alloc, json, .{}) catch return null;
    defer parsed.deinit();
    if (parsed.value != .object) return null;
    for (keys) |key| {
        const value = parsed.value.object.get(key) orelse continue;
        if (value != .string or value.string.len == 0) continue;
        return alloc.dupe(u8, value.string) catch null;
    }
    return null;
}

test "a bundle's dataName wins over its product name" {
    const name = nameFrom(std.testing.allocator, "{\"name\":\"Lynk Browser\",\"dataName\":\"lynk\"}", &manifest_keys).?;
    defer std.testing.allocator.free(name);
    try std.testing.expectEqualStrings("lynk", name);
}

test "an older bundle without dataName falls back to its name" {
    const name = nameFrom(std.testing.allocator, "{\"name\":\"Gallery\"}", &manifest_keys).?;
    defer std.testing.allocator.free(name);
    try std.testing.expectEqualStrings("Gallery", name);
}

test "a package.json without a name, or not JSON, gives nothing" {
    try std.testing.expect(nameFrom(std.testing.allocator, "{\"version\":\"1.0.0\"}", &package_keys) == null);
    try std.testing.expect(nameFrom(std.testing.allocator, "{\"name\":\"\"}", &package_keys) == null);
    try std.testing.expect(nameFrom(std.testing.allocator, "not json", &package_keys) == null);
}
