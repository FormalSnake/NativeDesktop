const std = @import("std");
const build_options = @import("build_options");

// The comptime backend seam: `tree.zig` calls `backend.impl.<fn>` instead of
// importing a concrete backend directly. The conformance target selects
// `null` to drive the in-memory backend (no C round-trip, fast, display-
// free); every other value routes through the C-ABI vtable seam (M6a-D1/D5)
// — "abi" (and any real embedder, including the GTK embedder once Task 5/6
// re-plumb it) forwards each seam call to a registered `nd_backend` vtable.
pub const impl = if (std.mem.eql(u8, build_options.backend, "null"))
    @import("null_backend.zig")
else
    @import("abi_backend.zig");
