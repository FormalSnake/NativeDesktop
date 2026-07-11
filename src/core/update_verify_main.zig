const std = @import("std");
const update = @import("update.zig");

fn readAll(io: std.Io, path: []const u8, gpa: std.mem.Allocator) ![]u8 {
    return std.Io.Dir.cwd().readFileAlloc(io, path, gpa, .limited(64 << 20));
}

fn decodeMinisignSecondLine(gpa: std.mem.Allocator, file: []const u8) ![]u8 {
    var it = std.mem.splitScalar(u8, file, '\n');
    _ = it.next();
    const b64 = std.mem.trim(u8, it.next() orelse return error.BadFormat, " \r\t");
    const dec = std.base64.standard.Decoder;
    const n = try dec.calcSizeForSlice(b64);
    const out = try gpa.alloc(u8, n);
    try dec.decode(out, b64);
    return out;
}

const Args = struct {
    pubkey_path: []const u8,
    message_path: []const u8,
    sig_path: []const u8,
};

fn parseArgs(argv: []const [*:0]const u8) ?Args {
    var pubkey_path: ?[]const u8 = null;
    var message_path: ?[]const u8 = null;
    var sig_path: ?[]const u8 = null;

    var i: usize = 1;
    while (i < argv.len) : (i += 1) {
        const arg = std.mem.span(argv[i]);
        if (std.mem.eql(u8, arg, "--pubkey") and i + 1 < argv.len) {
            i += 1;
            pubkey_path = std.mem.span(argv[i]);
        } else if (std.mem.eql(u8, arg, "--message") and i + 1 < argv.len) {
            i += 1;
            message_path = std.mem.span(argv[i]);
        } else if (std.mem.eql(u8, arg, "--sig") and i + 1 < argv.len) {
            i += 1;
            sig_path = std.mem.span(argv[i]);
        } else {
            return null;
        }
    }

    return Args{
        .pubkey_path = pubkey_path orelse return null,
        .message_path = message_path orelse return null,
        .sig_path = sig_path orelse return null,
    };
}

pub fn main(init: std.process.Init) !u8 {
    // page_allocator, not init.gpa: this is a one-shot bytes-in/exit-code-out
    // CLI with no frees before process exit, and init.gpa's debug build is
    // leak-checked — it would print spurious "leaked" noise on every run.
    const gpa = std.heap.page_allocator;
    const io = init.io;

    const args = parseArgs(init.minimal.args.vector) orelse {
        std.debug.print("usage: nd-update-verify --pubkey <f> --message <f> --sig <f>\n", .{});
        return 2;
    };

    const pk_file = try readAll(io, args.pubkey_path, gpa);
    const msg = try readAll(io, args.message_path, gpa);
    const sig_file = try readAll(io, args.sig_path, gpa);
    const pk = try decodeMinisignSecondLine(gpa, pk_file);
    const sig = try decodeMinisignSecondLine(gpa, sig_file);

    if (update.verifyMinisign(pk, msg, sig)) {
        std.debug.print("ND_UPDATE_VERIFY_OK\n", .{});
        return 0;
    }
    std.debug.print("ND_UPDATE_VERIFY_FAIL\n", .{});
    return 1;
}
