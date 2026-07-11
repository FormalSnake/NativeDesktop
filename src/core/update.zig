const std = @import("std");

pub const Manifest = struct {
    app_id: []const u8,
    version: []const u8,
    from: ?[]const u8 = null,
    full_url: []const u8,
    full_sig_b64: []const u8,
};

/// Verify a minisign signature blob over `message` with a minisign public-key blob.
/// STUB (T1): always false. T2 implements Ed25519 + Blake2b512-prehash verification.
pub fn verifyMinisign(pubkey_blob: []const u8, message: []const u8, sig_blob: []const u8) bool {
    _ = pubkey_blob;
    _ = message;
    _ = sig_blob;
    return false;
}

/// Parse the update manifest JSON. STUB (T1): T2 implements with std.json.
pub fn parseManifest(gpa: std.mem.Allocator, json: []const u8) !Manifest {
    _ = gpa;
    _ = json;
    return error.NotImplemented;
}

test "update stub compiles" {
    try std.testing.expect(!verifyMinisign("", "", ""));
}
