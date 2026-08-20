// Types shared across the CEF engine's compiled-in/compiled-out arms, so
// `backend.zig` can name them without reaching into either.
const protocol = @import("../protocol.zig");

/// Peer of `gtk/webview.zig`'s EmitFn. Function pointer types are structural in
/// Zig, so this IS that type as long as both name the same `protocol` file.
pub const EmitFn = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;

/// Live page state for the `webviewInfo` automation RPC. Unlike the WebKit
/// backend, which asks the engine, these are the last values the CEF handlers
/// pushed: every CEF read would have to cross to the CEF UI thread and back.
pub const Info = struct {
    url: ?[]const u8,
    title: ?[]const u8,
    loading: bool,
    can_go_back: bool,
    can_go_forward: bool,
};
