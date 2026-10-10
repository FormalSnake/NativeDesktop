//! Whether a page's silence can be held against its renderer. Chromium stops
//! passing devtools messages to the page while a main-frame navigation is in
//! flight (RenderFrameDevToolsAgentHost suspends its sessions from the start of
//! the navigation to its finish) and the frame may move to a new renderer when
//! it commits, so a ping sent across a navigation goes unanswered by a page
//! that is fine: a slow server, a stalled DNS lookup or connection, or a
//! cross-site swap all look hung to a watcher that ignores them.
//!
//! The navigation side is written on the CEF UI thread and read on the GTK one.
const std = @import("std");

pub const NavGate = struct {
    pending: std.atomic.Value(bool) = .init(false),
    /// Bumped at every start and finish, so a ping can tell that a navigation
    /// happened while it was out, even one too quick for a watch tick to see.
    epoch: std.atomic.Value(u32) = .init(0),

    /// A main-frame navigation (or a redirect of it) is under way.
    pub fn started(self: *NavGate) void {
        self.pending.store(true, .release);
        _ = self.epoch.fetchAdd(1, .acq_rel);
    }

    /// It committed, failed, or was abandoned.
    pub fn finished(self: *NavGate) void {
        self.pending.store(false, .release);
        _ = self.epoch.fetchAdd(1, .acq_rel);
    }

    pub fn navigating(self: *const NavGate) bool {
        return self.pending.load(.acquire);
    }

    /// Taken when a ping goes out.
    pub fn mark(self: *const NavGate) u32 {
        return self.epoch.load(.acquire);
    }

    /// An unanswered ping is a verdict only when no navigation started or
    /// finished since it went out and none is under way now.
    pub fn silenceCounts(self: *const NavGate, sent: u32) bool {
        return !self.navigating() and self.mark() == sent;
    }
};

test "a ping out across a stalled navigation is not a verdict" {
    var gate: NavGate = .{};
    const sent = gate.mark();
    gate.started();
    try std.testing.expect(gate.navigating());
    try std.testing.expect(!gate.silenceCounts(sent));
    try std.testing.expect(!gate.silenceCounts(gate.mark()));
}

test "a ping dropped by a navigation that already committed is not a verdict" {
    var gate: NavGate = .{};
    const sent = gate.mark();
    gate.started();
    gate.finished();
    try std.testing.expect(!gate.navigating());
    try std.testing.expect(!gate.silenceCounts(sent));
}

test "a redirect keeps the navigation pending" {
    var gate: NavGate = .{};
    gate.started();
    gate.started();
    try std.testing.expect(gate.navigating());
    gate.finished();
    try std.testing.expect(!gate.navigating());
}

test "silence after the page committed counts" {
    var gate: NavGate = .{};
    gate.started();
    gate.finished();
    const sent = gate.mark();
    try std.testing.expect(gate.silenceCounts(sent));
}
