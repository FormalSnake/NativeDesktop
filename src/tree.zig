const std = @import("std");
const gtk = @import("gtk");
const protocol = @import("protocol.zig");
const backend = @import("gtk_backend.zig");

pub const Tree = struct {
    gpa: std.mem.Allocator,
    app: *gtk.Application,
    nodes: std.AutoHashMapUnmanaged(u32, *gtk.Widget) = .{},
    generation: u32 = 0,

    pub fn init(gpa: std.mem.Allocator, app: *gtk.Application) Tree {
        return .{ .gpa = gpa, .app = app };
    }

    pub fn get(self: *Tree, id: u32) ?*gtk.Widget {
        return self.nodes.get(id);
    }

    /// UI-thread only. Applies an entire commit batch as one unit.
    pub fn apply(self: *Tree, batch: protocol.CommitBatch) void {
        self.generation = batch.generation;
        for (batch.ops) |op| {
            if (std.mem.eql(u8, op.op, "create")) {
                const widget = backend.createWidget(self.app, op.widget.?, op.props) catch continue;
                if (std.mem.eql(u8, op.widget.?, "Button")) {
                    backend.connectButtonClick(@ptrCast(@alignCast(widget)), op.id.?);
                }
                self.nodes.put(self.gpa, op.id.?, widget) catch continue;
            } else if (std.mem.eql(u8, op.op, "append")) {
                const parent = self.nodes.get(op.parent.?) orelse continue;
                const child = self.nodes.get(op.child.?) orelse continue;
                backend.appendChild(parent, child);
            } else if (std.mem.eql(u8, op.op, "setText")) {
                const widget = self.nodes.get(op.id.?) orelse continue;
                backend.setText(widget, op.text.?);
            } else if (std.mem.eql(u8, op.op, "update")) {
                const widget = self.nodes.get(op.id.?) orelse continue;
                backend.applyProps(widget, op.widget orelse "", op.props);
            } else {
                std.debug.print("ND_WARN unknown op={s}\n", .{op.op});
            }
        }
        std.debug.print("ND_COMMIT_APPLIED commitId={d}\n", .{batch.commitId});
    }
};
