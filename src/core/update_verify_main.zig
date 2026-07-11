const std = @import("std");
const update = @import("update.zig");

pub fn main() !u8 {
    // STUB (T1): T2 implements arg parsing + file reads (std.Io.Dir.readFileAlloc)
    // + verifyMinisign + exit-code contract (0 valid / 1 invalid / 2 usage).
    std.debug.print("ND_UPDATE_VERIFY stub\n", .{});
    return 2;
}
