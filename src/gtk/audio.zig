// GTK/Linux host-side of the `audio.*` system-capability family: playback +
// spectrum analysis on top of GStreamer's `playbin`. GStreamer is deliberately
// NOT a link-time dependency (same house rule as webview.zig's WebKitGTK and
// system.zig's libsecret): the ~dozen C entry points are resolved once with
// std.DynLib, so build.zig stays untouched and an absent runtime degrades to a
// clean "audio unavailable: gstreamer not found" instead of failing to link.
//
// Threading: every method here is dispatched from system.zig's `handleRequest`,
// which runs on the GTK UI thread (runtime.zig marshals `system_request`). The
// per-pipeline bus watch (gst_bus_add_watch_full) integrates with the GLib main
// loop already running on that same thread, so its callbacks also fire on the
// UI thread. The handle registry is therefore touched from one thread only — no
// locking needed.
const std = @import("std");
const glib = @import("glib");
const gobject = @import("gobject");
const abi = @import("../abi.zig");

const alloc = std.heap.page_allocator;

// Set at the top of every dispatched audio call. The bus watch callbacks fire
// outside a `system_request` (asynchronously, on the same UI thread), so they
// read the context back from here to push `nd_system_event`s.
var the_ctx: ?*abi.NdContext = null;

// ============================================================================
// GStreamer constants (stable across the whole 1.x ABI)
// ============================================================================

// GstState (gst/gstelement.h)
const GST_STATE_NULL: c_int = 1;
const GST_STATE_PAUSED: c_int = 3;
const GST_STATE_PLAYING: c_int = 4;
// GstStateChangeReturn
const GST_STATE_CHANGE_FAILURE: c_int = 0;
// GstFormat
const GST_FORMAT_TIME: c_int = 3;
// GstSeekFlags: FLUSH (1<<0) | KEY_UNIT (1<<2) — a flushing seek to the nearest
// keyframe, the usual choice for a UI scrub.
const GST_SEEK_FLAG_FLUSH_KEY_UNIT: c_int = (1 << 0) | (1 << 2);
// GstMessageType bits (gst/gstmessage.h) — each message carries exactly one.
const GST_MSG_EOS: c_uint = 1 << 0;
const GST_MSG_ERROR: c_uint = 1 << 1;
const GST_MSG_STATE_CHANGED: c_uint = 1 << 6;
const GST_MSG_ELEMENT: c_uint = 1 << 15;
// G_PRIORITY_DEFAULT (glib)
const G_PRIORITY_DEFAULT: c_int = 0;

// Fundamental GTypes (gobject/gtype.h: id << G_TYPE_FUNDAMENTAL_SHIFT, shift=2).
// Used to init a GValue before handing a property to g_object_set_property —
// this avoids the variadic g_object_set ABI risk entirely (mirrors the
// non-variadic discipline in system.zig's libsecret binding). The GstElement
// GType for the audio-filter property is resolved at runtime instead
// (gst_element_get_type), so it is never hardcoded.
const G_TYPE_BOOLEAN: usize = 5 << 2;
const G_TYPE_INT: usize = 6 << 2;
const G_TYPE_UINT: usize = 7 << 2;
const G_TYPE_UINT64: usize = 11 << 2;
const G_TYPE_DOUBLE: usize = 15 << 2;
const G_TYPE_STRING: usize = 16 << 2;

// Spectrum element configuration. 128 linear bands are folded into 32 log-spaced
// output bins; a 66.67 ms interval caps pushes at ~15 Hz; magnitudes below the
// -60 dB threshold read as silence.
const SPECTRUM_BANDS: c_uint = 128;
const SPECTRUM_INTERVAL_NS: u64 = 66_666_666;
const SPECTRUM_THRESHOLD_DB: c_int = -60;
const OUTPUT_BINS: usize = 32;

// Nominal Nyquist used to map linear band index -> frequency for the log fold.
// The spectrum ELEMENT message doesn't carry the sample rate, so a 44.1 kHz
// assumption is used; this is a visualization, not a measurement, so an exact
// rate isn't required.
const NOMINAL_NYQUIST_HZ: f64 = 22050.0;
const SPECTRUM_LO_HZ: f64 = 50.0;
const SPECTRUM_HI_HZ: f64 = 16000.0;

// The public prefix of GstMessage — public fields per gst/gstmessage.h and
// gst/gstminiobject.h, ABI-frozen for all of GStreamer 1.x. There is no
// function accessor for the message type (GST_MESSAGE_TYPE is macro-only), so
// the `type` and `src` fields are read directly; the comptime asserts pin the
// offsets against 64-bit layout drift.
const GstMiniObject = extern struct {
    typ: usize,
    refcount: c_int,
    lockstate: c_int,
    flags: c_uint,
    copy: ?*anyopaque,
    dispose: ?*anyopaque,
    free_fn: ?*anyopaque,
    n_qdata: c_uint,
    qdata: ?*anyopaque,
};

const GstMessage = extern struct {
    mini_object: GstMiniObject,
    typ: c_uint,
    timestamp: u64,
    src: ?*anyopaque,
};

comptime {
    std.debug.assert(@sizeOf(GstMiniObject) == 64);
    std.debug.assert(@offsetOf(GstMessage, "typ") == 64);
    std.debug.assert(@offsetOf(GstMessage, "src") == 80);
}

// g_source_remove lives in glib (linked into this host); declared locally so
// the bus watch can be torn down by its source id on an explicit stop.
extern fn g_source_remove(tag: c_uint) c_int;

// ============================================================================
// GStreamer dlopen table (lazy, one-time init — mirrors system.zig's loadSecret)
// ============================================================================

const GstBusFunc = *const fn (?*anyopaque, ?*anyopaque, ?*anyopaque) callconv(.c) c_int;

const FnInit = *const fn (?*anyopaque, ?*anyopaque) callconv(.c) void;
const FnMake = *const fn (?[*:0]const u8, ?[*:0]const u8) callconv(.c) ?*anyopaque;
const FnSetState = *const fn (?*anyopaque, c_int) callconv(.c) c_int;
const FnQuery = *const fn (?*anyopaque, c_int, *i64) callconv(.c) c_int;
const FnSeek = *const fn (?*anyopaque, c_int, c_int, i64) callconv(.c) c_int;
const FnGetBus = *const fn (?*anyopaque) callconv(.c) ?*anyopaque;
const FnGetType = *const fn () callconv(.c) usize;
const FnAddWatch = *const fn (?*anyopaque, c_int, GstBusFunc, ?*anyopaque, ?*anyopaque) callconv(.c) c_uint;
const FnGetStructure = *const fn (?*anyopaque) callconv(.c) ?*anyopaque;
const FnParseError = *const fn (?*anyopaque, *?*glib.Error, ?*?[*:0]u8) callconv(.c) void;
const FnParseStateChanged = *const fn (?*anyopaque, *c_int, *c_int, *c_int) callconv(.c) void;
const FnStructGetName = *const fn (?*anyopaque) callconv(.c) ?[*:0]const u8;
const FnStructGetValue = *const fn (?*anyopaque, [*:0]const u8) callconv(.c) ?*const anyopaque;
const FnListSize = *const fn (?*const anyopaque) callconv(.c) c_uint;
const FnListGetValue = *const fn (?*const anyopaque, c_uint) callconv(.c) ?*const anyopaque;
const FnFilenameToUri = *const fn ([*:0]const u8, ?*?*glib.Error) callconv(.c) ?[*:0]u8;

const GstApi = struct {
    init: FnInit,
    make: FnMake,
    set_state: FnSetState,
    query_position: FnQuery,
    query_duration: FnQuery,
    seek_simple: FnSeek,
    get_bus: FnGetBus,
    element_get_type: FnGetType,
    add_watch: FnAddWatch,
    get_structure: FnGetStructure,
    parse_error: FnParseError,
    parse_state_changed: FnParseStateChanged,
    struct_get_name: FnStructGetName,
    struct_get_value: FnStructGetValue,
    list_size: FnListSize,
    list_get_value: FnListGetValue,
    // gst_filename_to_uri percent-encodes a path into a file:// URI; optional
    // (a manual "file://" prefix is the fallback).
    filename_to_uri: ?FnFilenameToUri,
};

var gst_api: ?GstApi = null;
var gst_load_attempted = false;
// Held open for the process lifetime — GStreamer spawns registry/plugin state
// that isn't safe to unload once initialized.
var gst_lib: std.DynLib = undefined;

fn lookupAll(lib: *std.DynLib) ?GstApi {
    return .{
        .init = lib.lookup(FnInit, "gst_init") orelse return null,
        .make = lib.lookup(FnMake, "gst_element_factory_make") orelse return null,
        .set_state = lib.lookup(FnSetState, "gst_element_set_state") orelse return null,
        .query_position = lib.lookup(FnQuery, "gst_element_query_position") orelse return null,
        .query_duration = lib.lookup(FnQuery, "gst_element_query_duration") orelse return null,
        .seek_simple = lib.lookup(FnSeek, "gst_element_seek_simple") orelse return null,
        .get_bus = lib.lookup(FnGetBus, "gst_element_get_bus") orelse return null,
        .element_get_type = lib.lookup(FnGetType, "gst_element_get_type") orelse return null,
        .add_watch = lib.lookup(FnAddWatch, "gst_bus_add_watch_full") orelse return null,
        .get_structure = lib.lookup(FnGetStructure, "gst_message_get_structure") orelse return null,
        .parse_error = lib.lookup(FnParseError, "gst_message_parse_error") orelse return null,
        .parse_state_changed = lib.lookup(FnParseStateChanged, "gst_message_parse_state_changed") orelse return null,
        .struct_get_name = lib.lookup(FnStructGetName, "gst_structure_get_name") orelse return null,
        .struct_get_value = lib.lookup(FnStructGetValue, "gst_structure_get_value") orelse return null,
        .list_size = lib.lookup(FnListSize, "gst_value_list_get_size") orelse return null,
        .list_get_value = lib.lookup(FnListGetValue, "gst_value_list_get_value") orelse return null,
        .filename_to_uri = lib.lookup(FnFilenameToUri, "gst_filename_to_uri"),
    };
}

fn loadGst() ?*const GstApi {
    if (gst_load_attempted) return if (gst_api != null) &gst_api.? else null;
    gst_load_attempted = true;
    // The .0 soname is what distros ship; the dylib variants cover a brew/mac
    // GTK stack that also carries GStreamer.
    const candidates = [_][]const u8{
        "libgstreamer-1.0.so.0",
        "libgstreamer-1.0.so",
        "libgstreamer-1.0.0.dylib",
        "libgstreamer-1.0.dylib",
    };
    for (candidates) |name| {
        var lib = std.DynLib.open(name) catch continue;
        if (lookupAll(&lib)) |a| {
            gst_lib = lib;
            gst_api = a;
            a.init(null, null); // gst_init(NULL, NULL) — once, before any element
            std.debug.print("ND_AUDIO_ENGINE gstreamer ({s})\n", .{name});
            return &gst_api.?;
        }
        lib.close();
    }
    return null;
}

// ============================================================================
// Handle registry
// ============================================================================

const StateTag = enum { none, playing, paused };

const Player = struct {
    handle: []u8, // owned "audio-<n>"; also the registry key (no separate copy)
    playbin: *anyopaque,
    watch_id: c_uint,
    spectrum: bool, // spectrum events wanted AND the element attached
    last_state: StateTag = .none,
    duration_ms: i64 = -1, // < 0 until GStreamer knows the duration
};

var registry: std.StringHashMapUnmanaged(*Player) = .{};
var handle_counter: u64 = 0;

// ============================================================================
// Dispatch
// ============================================================================

/// Routes the `audio.*` family (system.zig's dispatcher hands every method
/// whose name starts with "audio."). Stores the context for the async bus-watch
/// event path.
pub fn handleRequest(ctx: *abi.NdContext, id: u32, method: []const u8, p: []const u8) void {
    the_ctx = ctx;
    if (std.mem.eql(u8, method, "audio.play")) return play(ctx, id, p);
    if (std.mem.eql(u8, method, "audio.pause")) return changeState(ctx, id, p, GST_STATE_PAUSED);
    if (std.mem.eql(u8, method, "audio.resume")) return changeState(ctx, id, p, GST_STATE_PLAYING);
    if (std.mem.eql(u8, method, "audio.stop")) return stop(ctx, id, p);
    if (std.mem.eql(u8, method, "audio.seek")) return seek(ctx, id, p);
    if (std.mem.eql(u8, method, "audio.setVolume")) return setVolume(ctx, id, p);
    respond(ctx, id, false, "unknown audio method");
}

// ============================================================================
// Methods
// ============================================================================

fn play(ctx: *abi.NdContext, id: u32, p: []const u8) void {
    const gst = loadGst() orelse return respond(ctx, id, false, "audio unavailable: gstreamer not found");
    const P = struct {
        path: ?[]const u8 = null,
        url: ?[]const u8 = null,
        volume: ?f64 = null,
        spectrum: bool = false,
    };
    const parsed = parseParams(P, p) orelse return respond(ctx, id, false, "invalid params");
    defer parsed.deinit();
    const v = parsed.value;

    const has_path = v.path != null and v.path.?.len > 0;
    const has_url = v.url != null and v.url.?.len > 0;
    if (has_path == has_url) return respond(ctx, id, false, "audio.play requires exactly one of path or url");

    const uri_z: [:0]u8 = if (has_url)
        (alloc.dupeZ(u8, v.url.?) catch return respond(ctx, id, false, "oom"))
    else
        (pathToUri(gst, v.path.?) orelse return respond(ctx, id, false, "oom"));
    defer alloc.free(uri_z);

    const playbin = gst.make("playbin", "nd-playbin") orelse return respond(ctx, id, false, "audio unavailable: playbin element missing");
    // Past this point any early return must unref `playbin` (its one floating
    // ref is our owning ref, dropped by g_object_unref at teardown).
    setStringProp(playbin, "uri", uri_z.ptr);
    setDoubleProp(playbin, "volume", clampVol(v.volume orelse 1.0));

    const spectrum_active = if (v.spectrum) attachSpectrum(gst, playbin) else false;

    handle_counter += 1;
    const handle = std.fmt.allocPrint(alloc, "audio-{d}", .{handle_counter}) catch {
        objUnref(playbin);
        return respond(ctx, id, false, "oom");
    };
    const player = alloc.create(Player) catch {
        alloc.free(handle);
        objUnref(playbin);
        return respond(ctx, id, false, "oom");
    };
    player.* = .{ .handle = handle, .playbin = playbin, .watch_id = 0, .spectrum = spectrum_active };

    const bus = gst.get_bus(playbin) orelse {
        alloc.destroy(player);
        alloc.free(handle);
        objUnref(playbin);
        return respond(ctx, id, false, "audio: pipeline has no bus");
    };
    player.watch_id = gst.add_watch(bus, G_PRIORITY_DEFAULT, &busCb, player, null);
    objUnref(bus); // the watch holds its own ref on the bus

    registry.put(alloc, handle, player) catch {
        _ = g_source_remove(player.watch_id);
        alloc.destroy(player);
        alloc.free(handle);
        objUnref(playbin);
        return respond(ctx, id, false, "oom");
    };

    if (gst.set_state(playbin, GST_STATE_PLAYING) == GST_STATE_CHANGE_FAILURE) {
        cleanup(gst, player, true);
        return respond(ctx, id, false, "audio: failed to start playback");
    }
    // The "playing" transition is emitted from the bus state-changed watch, not
    // here (set_state usually returns ASYNC while the pipeline prerolls).
    respondString(ctx, id, handle);
}

fn changeState(ctx: *abi.NdContext, id: u32, p: []const u8, state: c_int) void {
    const gst = loadGst() orelse return respond(ctx, id, false, "audio unavailable: gstreamer not found");
    const player = lookupParam(ctx, id, p) orelse return;
    _ = gst.set_state(player.playbin, state); // event follows from the bus watch
    respond(ctx, id, true, "null");
}

fn stop(ctx: *abi.NdContext, id: u32, p: []const u8) void {
    const gst = loadGst() orelse return respond(ctx, id, false, "audio unavailable: gstreamer not found");
    const player = lookupParam(ctx, id, p) orelse return;
    emitState(gst, player, "stopped", null); // query position/duration before teardown
    cleanup(gst, player, true); // releases the handle
    respond(ctx, id, true, "null");
}

fn seek(ctx: *abi.NdContext, id: u32, p: []const u8) void {
    const gst = loadGst() orelse return respond(ctx, id, false, "audio unavailable: gstreamer not found");
    const P = struct { handle: []const u8 = "", position: ?f64 = null };
    const parsed = parseParams(P, p) orelse return respond(ctx, id, false, "invalid params");
    defer parsed.deinit();
    const player = registry.get(parsed.value.handle) orelse return respond(ctx, id, false, "unknown audio handle");
    const ms: i64 = @intFromFloat(@max(parsed.value.position orelse 0, 0));
    _ = gst.seek_simple(player.playbin, GST_FORMAT_TIME, GST_SEEK_FLAG_FLUSH_KEY_UNIT, ms * std.time.ns_per_ms);
    respond(ctx, id, true, "null");
}

fn setVolume(ctx: *abi.NdContext, id: u32, p: []const u8) void {
    _ = loadGst() orelse return respond(ctx, id, false, "audio unavailable: gstreamer not found");
    const P = struct { handle: []const u8 = "", volume: ?f64 = null };
    const parsed = parseParams(P, p) orelse return respond(ctx, id, false, "invalid params");
    defer parsed.deinit();
    const player = registry.get(parsed.value.handle) orelse return respond(ctx, id, false, "unknown audio handle");
    setDoubleProp(player.playbin, "volume", clampVol(parsed.value.volume orelse 1.0));
    respond(ctx, id, true, "null");
}

/// Parses a `{handle}` param and resolves the live player, answering the request
/// itself (invalid params / unknown handle) when it can't.
fn lookupParam(ctx: *abi.NdContext, id: u32, p: []const u8) ?*Player {
    const P = struct { handle: []const u8 = "" };
    const parsed = parseParams(P, p) orelse {
        respond(ctx, id, false, "invalid params");
        return null;
    };
    defer parsed.deinit();
    return registry.get(parsed.value.handle) orelse {
        respond(ctx, id, false, "unknown audio handle");
        return null;
    };
}

/// Tears a pipeline down and releases its handle. `remove_watch` is true for the
/// explicit-stop path; false when called from inside the bus watch, where
/// returning G_SOURCE_REMOVE already detaches the source.
fn cleanup(gst: *const GstApi, player: *Player, remove_watch: bool) void {
    _ = gst.set_state(player.playbin, GST_STATE_NULL);
    if (remove_watch) _ = g_source_remove(player.watch_id);
    objUnref(player.playbin);
    _ = registry.remove(player.handle);
    alloc.free(player.handle);
    alloc.destroy(player);
}

// ============================================================================
// Spectrum
// ============================================================================

/// Creates and wires a `spectrum` element as playbin's audio-filter. Returns
/// false (playing continues without spectrum) if the element isn't in the
/// runtime's plugin set.
fn attachSpectrum(gst: *const GstApi, playbin: *anyopaque) bool {
    const spectrum = gst.make("spectrum", "nd-spectrum") orelse {
        std.debug.print("ND_WARN audio spectrum element unavailable; playing without spectrum\n", .{});
        return false;
    };
    // Claim the factory's floating ref so the round-trip through the
    // audio-filter property (which takes its own ref) leaves playbin owning it
    // and us releasing ours — version-independent of whether the setter sinks.
    _ = gobject.Object.refSink(@ptrCast(@alignCast(spectrum)));
    setUintProp(spectrum, "bands", SPECTRUM_BANDS);
    setUint64Prop(spectrum, "interval", SPECTRUM_INTERVAL_NS);
    setBoolProp(spectrum, "post-messages", 1);
    setIntProp(spectrum, "threshold", SPECTRUM_THRESHOLD_DB);
    setObjectProp(playbin, "audio-filter", gst.element_get_type(), spectrum);
    objUnref(spectrum);
    return true;
}

fn handleSpectrum(gst: *const GstApi, player: *Player, msg: *anyopaque) void {
    const s = gst.get_structure(msg) orelse return;
    const name = gst.struct_get_name(s) orelse return;
    if (!std.mem.eql(u8, std.mem.span(name), "spectrum")) return;
    const mags = gst.struct_get_value(s, "magnitude") orelse return;
    var bins: [OUTPUT_BINS]f32 = undefined;
    foldSpectrum(gst, mags, &bins);
    emitSpectrum(player, bins[0..]);
}

/// Folds the linear magnitude bands (dB, ~ -60..0) into 32 log-spaced output
/// bins spanning ~50 Hz–16 kHz, normalized to 0..1 against the -60 dB threshold.
/// Each output bin takes the max dB of the linear bands whose frequency range it
/// covers (punchier than averaging for a visualizer).
fn foldSpectrum(gst: *const GstApi, mags: *const anyopaque, out: *[OUTPUT_BINS]f32) void {
    const n = gst.list_size(mags);
    if (n == 0) {
        @memset(out, 0);
        return;
    }
    const nf: f64 = @floatFromInt(n);
    const ratio = SPECTRUM_HI_HZ / SPECTRUM_LO_HZ;
    const threshold: f64 = @floatFromInt(SPECTRUM_THRESHOLD_DB);
    var b: usize = 0;
    while (b < OUTPUT_BINS) : (b += 1) {
        const f0 = SPECTRUM_LO_HZ * std.math.pow(f64, ratio, @as(f64, @floatFromInt(b)) / OUTPUT_BINS);
        const f1 = SPECTRUM_LO_HZ * std.math.pow(f64, ratio, @as(f64, @floatFromInt(b + 1)) / OUTPUT_BINS);
        var lo: usize = @intFromFloat(@floor(f0 / NOMINAL_NYQUIST_HZ * nf));
        var hi: usize = @intFromFloat(@ceil(f1 / NOMINAL_NYQUIST_HZ * nf));
        if (lo >= n) lo = n - 1;
        if (hi <= lo) hi = lo + 1;
        if (hi > n) hi = n;
        var max_db: f64 = -std.math.inf(f64);
        var i = lo;
        while (i < hi) : (i += 1) {
            const gv = gst.list_get_value(mags, @intCast(i)) orelse continue;
            const db: f64 = gobject.Value.getFloat(@ptrCast(@alignCast(gv)));
            if (db > max_db) max_db = db;
        }
        // (db - threshold) / -threshold: threshold dB -> 0, 0 dB -> 1.
        const norm = if (max_db == -std.math.inf(f64)) 0.0 else (max_db - threshold) / -threshold;
        out[b] = @floatCast(std.math.clamp(norm, 0.0, 1.0));
    }
}

// ============================================================================
// Bus watch (async, UI thread)
// ============================================================================

fn busCb(_: ?*anyopaque, msg: ?*anyopaque, user_data: ?*anyopaque) callconv(.c) c_int {
    const player: *Player = @ptrCast(@alignCast(user_data orelse return 0));
    const m = msg orelse return 1;
    const gst: *const GstApi = if (gst_api) |*a| a else return 1;
    switch (@as(*const GstMessage, @ptrCast(@alignCast(m))).typ) {
        GST_MSG_EOS => {
            emitState(gst, player, "ended", null);
            cleanup(gst, player, false);
            return 0; // detach the watch
        },
        GST_MSG_ERROR => {
            var gerr: ?*glib.Error = null;
            gst.parse_error(m, &gerr, null);
            const emsg: []const u8 = if (gerr) |e| (if (e.f_message) |mm| std.mem.span(mm) else "playback error") else "playback error";
            emitState(gst, player, "error", emsg);
            if (gerr) |e| e.free();
            cleanup(gst, player, false);
            return 0;
        },
        GST_MSG_STATE_CHANGED => {
            // Every element posts state-changed; only the top-level playbin's
            // own transition is a real "playing"/"paused" for the app.
            const src = @as(*const GstMessage, @ptrCast(@alignCast(m))).src;
            if (src != @as(?*anyopaque, player.playbin)) return 1;
            var olds: c_int = 0;
            var news: c_int = 0;
            var pend: c_int = 0;
            gst.parse_state_changed(m, &olds, &news, &pend);
            if (news == GST_STATE_PLAYING and player.last_state != .playing) {
                player.last_state = .playing;
                emitState(gst, player, "playing", null);
            } else if (news == GST_STATE_PAUSED and player.last_state != .paused) {
                player.last_state = .paused;
                emitState(gst, player, "paused", null);
            }
            return 1;
        },
        GST_MSG_ELEMENT => {
            if (player.spectrum) handleSpectrum(gst, player, m);
            return 1;
        },
        else => return 1,
    }
}

// ============================================================================
// Events
// ============================================================================

fn emitState(gst: *const GstApi, player: *Player, state: []const u8, err: ?[]const u8) void {
    const ctx = the_ctx orelse return;
    const pos = queryMs(gst, player.playbin, .position) orelse 0;
    if (queryMs(gst, player.playbin, .duration)) |d| player.duration_ms = d;
    const dur: ?i64 = if (player.duration_ms >= 0) player.duration_ms else null;
    emitEvent(ctx, "audio.state", .{
        .handle = player.handle,
        .state = state,
        .position = pos,
        .duration = dur,
        .@"error" = err,
    });
}

fn emitSpectrum(player: *Player, bins: []const f32) void {
    const ctx = the_ctx orelse return;
    emitEvent(ctx, "audio.spectrum", .{ .handle = player.handle, .bins = bins });
}

const QueryKind = enum { position, duration };

fn queryMs(gst: *const GstApi, element: *anyopaque, kind: QueryKind) ?i64 {
    var v: i64 = 0;
    const ok = switch (kind) {
        .position => gst.query_position(element, GST_FORMAT_TIME, &v),
        .duration => gst.query_duration(element, GST_FORMAT_TIME, &v),
    };
    if (ok == 0 or v < 0) return null;
    return @divTrunc(v, std.time.ns_per_ms);
}

// ============================================================================
// Helpers
// ============================================================================

fn respond(ctx: *abi.NdContext, id: u32, ok: bool, json: []const u8) void {
    const z = alloc.dupeZ(u8, json) catch return;
    defer alloc.free(z);
    abi.nd_system_response(ctx, id, ok, z.ptr);
}

fn respondString(ctx: *abi.NdContext, id: u32, s: []const u8) void {
    const json = std.json.Stringify.valueAlloc(alloc, s, .{}) catch return respond(ctx, id, false, "oom");
    defer alloc.free(json);
    respond(ctx, id, true, json);
}

fn emitEvent(ctx: *abi.NdContext, channel: []const u8, value: anytype) void {
    const json = std.json.Stringify.valueAlloc(alloc, value, .{}) catch return;
    defer alloc.free(json);
    const json_z = alloc.dupeZ(u8, json) catch return;
    defer alloc.free(json_z);
    const chan_z = alloc.dupeZ(u8, channel) catch return;
    defer alloc.free(chan_z);
    abi.nd_system_event(ctx, chan_z.ptr, json_z.ptr);
}

fn parseParams(comptime T: type, p: []const u8) ?std.json.Parsed(T) {
    return std.json.parseFromSlice(T, alloc, p, .{ .ignore_unknown_fields = true }) catch null;
}

fn clampVol(v: f64) f64 {
    return std.math.clamp(v, 0.0, 1.0);
}

/// `g_object_unref` on a GObject-derived opaque handle (mirrors system.zig's
/// objUnref — the opaque handles are alignment-1, so the cast to 8-aligned
/// Object needs the explicit @alignCast).
fn objUnref(obj: *anyopaque) void {
    gobject.Object.unref(@ptrCast(@alignCast(obj)));
}

fn pathToUri(gst: *const GstApi, path: []const u8) ?[:0]u8 {
    const path_z = alloc.dupeZ(u8, path) catch return null;
    defer alloc.free(path_z);
    if (gst.filename_to_uri) |f| {
        var err: ?*glib.Error = null;
        if (f(path_z.ptr, &err)) |uri_c| {
            defer glib.free(uri_c);
            return alloc.dupeZ(u8, std.mem.span(uri_c)) catch null;
        }
        if (err) |e| e.free();
    }
    // Fallback: assumes an absolute path (no percent-encoding).
    return std.fmt.allocPrintSentinel(alloc, "file://{s}", .{path}, 0) catch null;
}

// ============================================================================
// GObject property setters (GValue-based — no variadic g_object_set)
// ============================================================================

fn setStringProp(obj: *anyopaque, name: [*:0]const u8, val: [*:0]const u8) void {
    var v: gobject.Value = std.mem.zeroes(gobject.Value);
    _ = gobject.Value.init(&v, G_TYPE_STRING);
    gobject.Value.setString(&v, val);
    gobject.Object.setProperty(@ptrCast(@alignCast(obj)), name, &v);
    gobject.Value.unset(&v);
}

fn setDoubleProp(obj: *anyopaque, name: [*:0]const u8, val: f64) void {
    var v: gobject.Value = std.mem.zeroes(gobject.Value);
    _ = gobject.Value.init(&v, G_TYPE_DOUBLE);
    gobject.Value.setDouble(&v, val);
    gobject.Object.setProperty(@ptrCast(@alignCast(obj)), name, &v);
    gobject.Value.unset(&v);
}

fn setIntProp(obj: *anyopaque, name: [*:0]const u8, val: c_int) void {
    var v: gobject.Value = std.mem.zeroes(gobject.Value);
    _ = gobject.Value.init(&v, G_TYPE_INT);
    gobject.Value.setInt(&v, val);
    gobject.Object.setProperty(@ptrCast(@alignCast(obj)), name, &v);
    gobject.Value.unset(&v);
}

fn setUintProp(obj: *anyopaque, name: [*:0]const u8, val: c_uint) void {
    var v: gobject.Value = std.mem.zeroes(gobject.Value);
    _ = gobject.Value.init(&v, G_TYPE_UINT);
    gobject.Value.setUint(&v, val);
    gobject.Object.setProperty(@ptrCast(@alignCast(obj)), name, &v);
    gobject.Value.unset(&v);
}

fn setUint64Prop(obj: *anyopaque, name: [*:0]const u8, val: u64) void {
    var v: gobject.Value = std.mem.zeroes(gobject.Value);
    _ = gobject.Value.init(&v, G_TYPE_UINT64);
    gobject.Value.setUint64(&v, val);
    gobject.Object.setProperty(@ptrCast(@alignCast(obj)), name, &v);
    gobject.Value.unset(&v);
}

fn setBoolProp(obj: *anyopaque, name: [*:0]const u8, val: c_int) void {
    var v: gobject.Value = std.mem.zeroes(gobject.Value);
    _ = gobject.Value.init(&v, G_TYPE_BOOLEAN);
    gobject.Value.setBoolean(&v, val);
    gobject.Object.setProperty(@ptrCast(@alignCast(obj)), name, &v);
    gobject.Value.unset(&v);
}

fn setObjectProp(obj: *anyopaque, name: [*:0]const u8, gtype: usize, child: *anyopaque) void {
    var v: gobject.Value = std.mem.zeroes(gobject.Value);
    _ = gobject.Value.init(&v, gtype);
    gobject.Value.setObject(&v, @ptrCast(@alignCast(child)));
    gobject.Object.setProperty(@ptrCast(@alignCast(obj)), name, &v);
    gobject.Value.unset(&v);
}
