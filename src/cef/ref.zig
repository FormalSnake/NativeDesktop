// The capi refcount contract, once, for every CEF struct this engine
// implements. Open-coding it per handler is how the two failure modes get in:
// a `base.size` that does not match the struct CEF thinks it received (CEF
// reads past the end of the allocation), and a callback parameter left
// unreleased (a leaked browser holds a whole renderer process open).
//
// Ownership rules this encodes, from libcef_dll/cpptoc/cpptoc_ref_counted.h:
//   - Objects implemented here start at one reference, owned by whoever
//     created them; CEF add_refs when it stores one.
//   - A ref-counted parameter arriving in one of our callbacks carries a
//     reference the callback owns ("released once our structure arrives on the
//     other side"), so it is released before the callback returns unless the
//     callback deliberately keeps it.
//   - A ref-counted value returned FROM a callback (get_display_handler and
//     friends) is a reference the caller owns, so it is add_ref'd on the way
//     out.
//   - Passing an object INTO a CEF function transfers the caller's reference:
//     CefCToCpp::Wrap consumes it ("Release the reference that was added ...
//     before their structure was passed to us"). Anything still needed after
//     the call has to be add_ref'd first, which is what `handOut` is for, and
//     anything NOT needed afterwards must not also be released. Both halves of
//     that rule have cost this project a crash far from its cause.
const std = @import("std");
const capi = @import("capi.zig");
const c = capi.c;

/// libc's allocator, not page_allocator: these objects are small, numerous
/// enough to matter, and freed from the CEF UI thread as often as from the GTK
/// one.
pub const gpa = std.heap.c_allocator;

/// One CEF capi object implemented here. CEF only ever holds `&self.cef`, so
/// the container's own layout is unconstrained and `@fieldParentPtr` walks
/// back from the C struct to the Zig payload.
pub fn Counted(comptime CStruct: type, comptime Payload: type) type {
    return struct {
        const Self = @This();

        cef: CStruct,
        refs: std.atomic.Value(u32),
        payload: Payload,

        /// One reference, owned by the caller. Every function pointer beyond
        /// `base` starts null; the caller fills in the arms it implements.
        pub fn create(payload: Payload) ?*Self {
            const self = gpa.create(Self) catch return null;
            self.* = .{
                .cef = std.mem.zeroes(CStruct),
                .refs = .init(1),
                .payload = payload,
            };
            self.cef.base = .{
                .size = @sizeOf(CStruct),
                .add_ref = addRef,
                .release = release,
                .has_one_ref = hasOneRef,
                .has_at_least_one_ref = hasAtLeastOneRef,
            };
            return self;
        }

        /// One added reference, for a pointer about to cross into CEF: either
        /// returned from a `get_*_handler` arm or passed as an argument. Both
        /// directions consume what they are given.
        pub fn handOut(self: *Self) *CStruct {
            _ = self.refs.fetchAdd(1, .monotonic);
            return &self.cef;
        }

        pub fn cptr(self: *Self) *CStruct {
            return &self.cef;
        }

        /// Drops the reference `create` handed back. Used to unwind a
        /// half-built object; CEF has not seen it, so this is also the free.
        pub fn drop(self: *Self) void {
            _ = release(@ptrCast(&self.cef.base));
        }

        pub fn of(cef: anytype) *Self {
            const typed: *CStruct = @ptrCast(@alignCast(cef));
            return @fieldParentPtr("cef", typed);
        }

        fn fromBase(base: [*c]c.cef_base_ref_counted_t) *Self {
            const typed: *CStruct = @ptrCast(@alignCast(base));
            return @fieldParentPtr("cef", typed);
        }

        fn addRef(base: [*c]c.cef_base_ref_counted_t) callconv(.c) void {
            _ = fromBase(base).refs.fetchAdd(1, .monotonic);
        }

        fn release(base: [*c]c.cef_base_ref_counted_t) callconv(.c) c_int {
            const self = fromBase(base);
            // acq_rel, not monotonic: the thread that drops the last reference
            // must see every write the other threads made through it before
            // the memory goes back.
            if (self.refs.fetchSub(1, .acq_rel) == 1) {
                gpa.destroy(self);
                return 1;
            }
            return 0;
        }

        fn hasOneRef(base: [*c]c.cef_base_ref_counted_t) callconv(.c) c_int {
            return @intFromBool(fromBase(base).refs.load(.acquire) == 1);
        }

        fn hasAtLeastOneRef(base: [*c]c.cef_base_ref_counted_t) callconv(.c) c_int {
            return @intFromBool(fromBase(base).refs.load(.acquire) >= 1);
        }
    };
}

/// Drops the reference a callback parameter arrived with. Every ref-counted
/// parameter of every handler goes through this before the callback returns.
pub fn releaseParam(p: anytype) void {
    if (p == null) return;
    // Every CEF struct starts with its base at offset 0, so the object pointer
    // IS the base pointer; going through `&p.*.base` instead re-derives a C
    // pointer type Zig will not let the base's own methods be read off.
    const base: *c.cef_base_ref_counted_t = @ptrCast(@alignCast(p));
    if (base.release) |f| _ = f(base);
}

/// Drops a reference obtained from a CEF getter (`get_host`, `get_main_frame`),
/// the same contract as a callback parameter in the other direction.
pub const releaseOwned = releaseParam;
