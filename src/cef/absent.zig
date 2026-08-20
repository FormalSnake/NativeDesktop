// The CEF engine compiled out: every entry point answers "not me", which is
// what `gtk/webview.zig` needs to fall through to WebKitGTK. Kept byte-for-byte
// in signature with engine.zig so the seam in backend.zig stays a comptime
// choice rather than a set of call-site conditionals.
const std = @import("std");
const gtk = @import("gtk");
const types = @import("types.zig");

pub fn earlyExecuteProcess(_: []const [*:0]const u8) ?u8 {
    return null;
}

pub fn pinDisplayBackend() void {}

pub fn shutdown() void {}

pub fn create(_: ?[*:0]const u8, _: []const u8, _: []const u8) ?*gtk.Widget {
    return null;
}

pub fn isReal(_: *gtk.Widget) bool {
    return false;
}

pub fn setUrl(_: *gtk.Widget, _: [:0]const u8) void {}

pub fn command(_: *gtk.Widget, _: []const u8, _: ?std.json.Value) void {}

pub fn connectEvents(_: *gtk.Widget, _: u32, _: types.EmitFn) void {}

pub fn info(_: *gtk.Widget) ?types.Info {
    return null;
}

pub const EvalState = struct { done: bool, ok: bool, value: ?[]const u8, err: ?[]const u8 };

pub fn evalStart(_: *gtk.Widget, _: []const u8, _: ?[]const u8) ?u64 {
    return null;
}

pub fn evalPoll(_: u64) ?EvalState {
    return null;
}

pub fn evalRelease(_: u64) void {}

pub fn pageText(_: *gtk.Widget) ?[]const u8 {
    return null;
}
