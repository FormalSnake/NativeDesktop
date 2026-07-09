const std = @import("std");
const build_options = @import("build_options");

// The comptime backend seam: `tree.zig` calls `backend.impl.<fn>` instead of
// importing `gtk_backend.zig` directly. The shipping exe selects `gtk` (zero
// indirection, byte-identical to pre-seam behavior); the conformance target
// selects `null` to drive the in-memory backend. No runtime vtable.
pub const impl = if (std.mem.eql(u8, build_options.backend, "null"))
    @import("null_backend.zig")
else
    @import("gtk_backend.zig");
