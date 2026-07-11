const std = @import("std");
const Ed25519 = std.crypto.sign.Ed25519;
const Blake2b512 = std.crypto.hash.blake2.Blake2b512;

pub const Manifest = struct {
    app_id: []const u8,
    version: []const u8,
    from: ?[]const u8 = null,
    full_url: []const u8,
    full_sig_b64: []const u8,
};

const KEY_ID_LEN = 8;
const ALGO_LEN = 2;

/// Verify a minisign signature blob over `message` with a minisign public-key blob.
/// `pubkey_blob`/`sig_blob` are already base64-decoded (the second line of a
/// minisign `.pub`/`.minisig` file): `algo[2] ‖ key_id[8] ‖ pk_or_sig[N]`.
/// This is the non-disableable update-verification core (spec §11): pure
/// over byte slices, no network, no filesystem.
pub fn verifyMinisign(pubkey_blob: []const u8, message: []const u8, sig_blob: []const u8) bool {
    if (pubkey_blob.len != ALGO_LEN + KEY_ID_LEN + Ed25519.PublicKey.encoded_length) return false;
    if (sig_blob.len != ALGO_LEN + KEY_ID_LEN + Ed25519.Signature.encoded_length) return false;

    const pk_keyid = pubkey_blob[ALGO_LEN .. ALGO_LEN + KEY_ID_LEN];
    const sig_algo = sig_blob[0..ALGO_LEN];
    const sig_keyid = sig_blob[ALGO_LEN .. ALGO_LEN + KEY_ID_LEN];
    if (!std.mem.eql(u8, pk_keyid, sig_keyid)) return false; // wrong signer

    var pk_bytes: [Ed25519.PublicKey.encoded_length]u8 = undefined;
    @memcpy(&pk_bytes, pubkey_blob[ALGO_LEN + KEY_ID_LEN ..]);
    var sig_bytes: [Ed25519.Signature.encoded_length]u8 = undefined;
    @memcpy(&sig_bytes, sig_blob[ALGO_LEN + KEY_ID_LEN ..]);

    const pk = Ed25519.PublicKey.fromBytes(pk_bytes) catch return false;
    const sig = Ed25519.Signature.fromBytes(sig_bytes);

    // Algo tag: "Ed" (0x45,0x64) signs the raw message; "ED" (0x45,0x44) signs
    // Blake2b-512(message) (minisign's prehashed form for large files).
    if (sig_algo[0] == 'E' and sig_algo[1] == 'd') {
        sig.verify(message, pk) catch return false;
        return true;
    } else if (sig_algo[0] == 'E' and sig_algo[1] == 'D') {
        var digest: [Blake2b512.digest_length]u8 = undefined;
        Blake2b512.hash(message, &digest, .{});
        sig.verify(&digest, pk) catch return false;
        return true;
    }
    return false; // unknown algorithm tag
}

/// Parse the update manifest JSON into owned copies (gpa-allocated).
pub fn parseManifest(gpa: std.mem.Allocator, json: []const u8) !Manifest {
    const Parsed = struct {
        app_id: []const u8,
        version: []const u8,
        from: ?[]const u8 = null,
        full_url: []const u8,
        full_sig_b64: []const u8,
    };
    const parsed = try std.json.parseFromSlice(Parsed, gpa, json, .{ .ignore_unknown_fields = true });
    defer parsed.deinit();
    return Manifest{
        .app_id = try gpa.dupe(u8, parsed.value.app_id),
        .version = try gpa.dupe(u8, parsed.value.version),
        .from = if (parsed.value.from) |f| try gpa.dupe(u8, f) else null,
        .full_url = try gpa.dupe(u8, parsed.value.full_url),
        .full_sig_b64 = try gpa.dupe(u8, parsed.value.full_sig_b64),
    };
}

// Test fixtures under src/core/testdata/ are throwaway test-only minisign
// keys generated fresh for this repo (`minisign -G -W`, no password, secret
// key deleted immediately after signing) — they have no real-world value.
const test_pub_file = @embedFile("testdata/test.pub");
const valid_msg = @embedFile("testdata/msg-valid.txt");
const valid_sig_file = @embedFile("testdata/msg-valid.txt.minisig");
const tampered_msg = @embedFile("testdata/msg-tampered.txt");

// Decode the second (base64) line of a minisign .pub / .minisig file.
fn decodeMinisignBlob(alloc: std.mem.Allocator, file: []const u8) ![]u8 {
    var it = std.mem.splitScalar(u8, file, '\n');
    _ = it.next(); // untrusted/trusted comment line
    const b64 = std.mem.trim(u8, it.next() orelse return error.BadFormat, " \r\t");
    const dec = std.base64.standard.Decoder;
    const n = try dec.calcSizeForSlice(b64);
    const out = try alloc.alloc(u8, n);
    try dec.decode(out, b64);
    return out;
}

test "valid minisign signature verifies" {
    const a = std.testing.allocator;
    const pk = try decodeMinisignBlob(a, test_pub_file);
    defer a.free(pk);
    const sig = try decodeMinisignBlob(a, valid_sig_file);
    defer a.free(sig);
    try std.testing.expect(verifyMinisign(pk, valid_msg, sig));
}

test "tampered message is rejected" {
    const a = std.testing.allocator;
    const pk = try decodeMinisignBlob(a, test_pub_file);
    defer a.free(pk);
    const sig = try decodeMinisignBlob(a, valid_sig_file);
    defer a.free(sig);
    try std.testing.expect(!verifyMinisign(pk, tampered_msg, sig));
}

test "key_id mismatch is rejected" {
    const a = std.testing.allocator;
    const pk = try decodeMinisignBlob(a, test_pub_file);
    defer a.free(pk);
    const sig = try decodeMinisignBlob(a, valid_sig_file);
    defer a.free(sig);
    var bad_pk = try a.dupe(u8, pk);
    defer a.free(bad_pk);
    bad_pk[2] ^= 0xFF; // corrupt the key_id
    try std.testing.expect(!verifyMinisign(bad_pk, valid_msg, sig));
}

test "parseManifest reads app_id/version/full" {
    const a = std.testing.allocator;
    const json =
        \\{"app_id":"com.nativedesktop.gallery","version":"1.2.0","from":"1.1.0",
        \\ "full_url":"http://127.0.0.1:9/full.tar.zst","full_sig_b64":"AAAA"}
    ;
    const m = try parseManifest(a, json);
    defer a.free(m.app_id);
    defer a.free(m.version);
    defer if (m.from) |f| a.free(f);
    defer a.free(m.full_url);
    defer a.free(m.full_sig_b64);
    try std.testing.expectEqualStrings("com.nativedesktop.gallery", m.app_id);
    try std.testing.expectEqualStrings("1.2.0", m.version);
    try std.testing.expectEqualStrings("1.1.0", m.from.?);
}
