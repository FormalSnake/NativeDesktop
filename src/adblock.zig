//! Built-in content blocking: uBlock Origin's filter lists evaluated by
//! brave/adblock-rust (crates/nd-adblock, built by cargo from build.zig), for
//! the Chromium engine on both hosts.
//!
//! The app loads lists with `webviewEngine.contentBlockingLoad` and tunes them
//! with `webviewEngine.contentBlockingConfigure`; the hosts ask `decide` for
//! every network request and `serve` for every `nd-adblock:` request, both on
//! CEF's IO thread. Neither has an `Io`, so the state is one immutable
//! snapshot swapped under a spin lock and reference counted: a reader never
//! holds the lock while it matches.

const std = @import("std");
const marker = @import("marker.zig");

const gpa = std.heap.c_allocator;

// ---------------------------------------------------------------------------
// crates/nd-adblock C ABI
// ---------------------------------------------------------------------------

const RawEngine = opaque {};
extern fn nd_ab_engine_from_lists(lists: [*]const [*:0]const u8, formats: [*]const i32, count: usize) ?*RawEngine;
extern fn nd_ab_engine_deserialize(data: [*]const u8, len: usize) ?*RawEngine;
extern fn nd_ab_engine_serialize(e: *const RawEngine, out: *[*]u8, out_len: *usize) bool;
extern fn nd_ab_engine_use_resources(e: *RawEngine, json: [*]const u8, len: usize) bool;
extern fn nd_ab_engine_free(e: *RawEngine) void;
extern fn nd_ab_check(e: *const RawEngine, url: [*:0]const u8, source: [*:0]const u8, kind: [*:0]const u8, redirect: ?*?[*:0]u8) i32;
extern fn nd_ab_url_cosmetic(e: *const RawEngine, url: [*:0]const u8) ?[*:0]u8;
extern fn nd_ab_hidden_selectors(e: *const RawEngine, classes: [*:0]const u8, ids: [*:0]const u8, exceptions: [*:0]const u8) ?[*:0]u8;
extern fn nd_ab_string_free(s: ?[*:0]u8) void;
extern fn nd_ab_bytes_free(p: [*]u8, len: usize) void;

const AB_ALLOW = 0;
const AB_BLOCK = 1;
const AB_REDIRECT = 2;
const AB_EXCEPTED = 3;

/// One compiled engine, shared by every snapshot that points at it.
const Engine = struct {
    raw: *RawEngine,
    refs: std.atomic.Value(u32) = .init(1),

    fn create(raw: *RawEngine) ?*Engine {
        const e = gpa.create(Engine) catch {
            nd_ab_engine_free(raw);
            return null;
        };
        e.* = .{ .raw = raw };
        return e;
    }

    fn retain(e: *Engine) *Engine {
        _ = e.refs.fetchAdd(1, .monotonic);
        return e;
    }

    fn release(e: *Engine) void {
        if (e.refs.fetchSub(1, .acq_rel) == 1) {
            nd_ab_engine_free(e.raw);
            gpa.destroy(e);
        }
    }
};

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

const Snapshot = struct {
    refs: std.atomic.Value(u32) = .init(1),
    enabled: bool = true,
    lists: ?*Engine = null,
    /// The app's own rules (the element hider's `host##selector` lines, and
    /// anything else it lets the user type), compiled apart from the lists so
    /// an edit never recompiles the lists.
    user: ?*Engine = null,
    /// Hostnames the user switched blocking off for. A site matches its own
    /// entry and every parent domain's.
    disabled: std.StringHashMapUnmanaged(void) = .empty,

    fn release(s: *Snapshot) void {
        if (s.refs.fetchSub(1, .acq_rel) != 1) return;
        if (s.lists) |e| e.release();
        if (s.user) |e| e.release();
        var it = s.disabled.keyIterator();
        while (it.next()) |k| gpa.free(k.*);
        s.disabled.deinit(gpa);
        gpa.destroy(s);
    }

    /// A copy sharing the engines, for a configure call to edit.
    fn clone(s: *const Snapshot) !*Snapshot {
        const n = try gpa.create(Snapshot);
        n.* = .{ .enabled = s.enabled };
        errdefer n.release();
        n.lists = if (s.lists) |e| e.retain() else null;
        n.user = if (s.user) |e| e.retain() else null;
        var it = s.disabled.keyIterator();
        while (it.next()) |k| try n.disabled.put(gpa, try gpa.dupe(u8, k.*), {});
        return n;
    }

    fn siteDisabled(s: *const Snapshot, top_url: []const u8) bool {
        if (!s.enabled) return true;
        if (s.disabled.count() == 0) return false;
        var host = hostOf(top_url);
        while (host.len > 0) {
            if (s.disabled.contains(host)) return true;
            const dot = std.mem.indexOfScalar(u8, host, '.') orelse break;
            host = host[dot + 1 ..];
        }
        return false;
    }
};

const SpinLock = struct {
    held: std.atomic.Value(bool) = .init(false),

    fn lock(self: *SpinLock) void {
        while (self.held.cmpxchgWeak(false, true, .acquire, .monotonic) != null) std.Thread.yield() catch {};
    }

    fn unlock(self: *SpinLock) void {
        self.held.store(false, .release);
    }
};

var snap_lock: SpinLock = .{};
var current: ?*Snapshot = null;

fn acquire() ?*Snapshot {
    snap_lock.lock();
    defer snap_lock.unlock();
    const s = current orelse return null;
    _ = s.refs.fetchAdd(1, .monotonic);
    return s;
}

fn publish(next: *Snapshot) void {
    snap_lock.lock();
    const old = current;
    current = next;
    snap_lock.unlock();
    if (old) |o| o.release();
}

/// The snapshot a configure or load call starts from: the live one, or an
/// empty enabled one before the first load.
fn cloneCurrent() !*Snapshot {
    if (acquire()) |s| {
        defer s.release();
        return s.clone();
    }
    const n = try gpa.create(Snapshot);
    n.* = .{};
    return n;
}

// ---------------------------------------------------------------------------
// URL helpers
// ---------------------------------------------------------------------------

/// The hostname of an absolute URL, without userinfo or port, or "" when the
/// URL has no authority (about:blank, data:, a bare origin-less string).
pub fn hostOf(url: []const u8) []const u8 {
    const scheme_end = std.mem.indexOf(u8, url, "://") orelse return "";
    var rest = url[scheme_end + 3 ..];
    if (std.mem.indexOfAny(u8, rest, "/?#")) |end| rest = rest[0..end];
    if (std.mem.lastIndexOfScalar(u8, rest, '@')) |at| rest = rest[at + 1 ..];
    if (rest.len > 0 and rest[0] == '[') {
        const close = std.mem.indexOfScalar(u8, rest, ']') orelse return rest;
        return rest[0 .. close + 1];
    }
    if (std.mem.lastIndexOfScalar(u8, rest, ':')) |colon| rest = rest[0..colon];
    return rest;
}

fn isHttp(url: []const u8) bool {
    return std.mem.startsWith(u8, url, "http://") or std.mem.startsWith(u8, url, "https://") or
        std.mem.startsWith(u8, url, "ws://") or std.mem.startsWith(u8, url, "wss://");
}

// ---------------------------------------------------------------------------
// Network decisions (CEF IO thread)
// ---------------------------------------------------------------------------

/// cef_resource_type_t, as the hosts pass it through untouched.
pub const ResourceType = enum(c_int) {
    main_frame = 0,
    sub_frame = 1,
    stylesheet = 2,
    script = 3,
    image = 4,
    font_resource = 5,
    sub_resource = 6,
    object = 7,
    media = 8,
    worker = 9,
    shared_worker = 10,
    prefetch = 11,
    favicon = 12,
    xhr = 13,
    ping = 14,
    service_worker = 15,
    csp_report = 16,
    plugin_resource = 17,
    navigation_preload_main_frame = 19,
    navigation_preload_sub_frame = 20,
    _,
};

/// adblock-rust's request type names (uBO's), per CEF resource type.
fn kindName(t: ResourceType) [:0]const u8 {
    return switch (t) {
        .main_frame, .navigation_preload_main_frame => "document",
        .sub_frame, .navigation_preload_sub_frame => "sub_frame",
        .stylesheet => "stylesheet",
        .script, .worker, .shared_worker, .service_worker => "script",
        .image, .favicon => "image",
        .font_resource => "font",
        .object, .plugin_resource => "object",
        .media => "media",
        .xhr => "xmlhttprequest",
        .ping => "ping",
        .csp_report => "csp_report",
        .sub_resource, .prefetch => "other",
        _ => "other",
    };
}

pub const Action = enum(c_int) { allow = 0, block = 1, redirect = 2 };

pub const Decision = extern struct {
    action: Action = .allow,
    /// For `.redirect`: the body to serve and its MIME type, owned by the
    /// caller until `nd_adblock_decision_free`.
    body: ?[*]u8 = null,
    body_len: usize = 0,
    mime: ?[*:0]u8 = null,
};

/// Decides one request. `initiator` is the origin (or URL) of the document
/// that made it, `top_url` the tab's main-frame URL, which is what a per-site
/// switch keys on. Top-level documents are never blocked: a blocked page is a
/// dead tab, and uBO's strict-blocking interstitial is not built here.
pub fn decide(url: []const u8, initiator: []const u8, top_url: []const u8, kind: ResourceType) Decision {
    if (kind == .main_frame or kind == .navigation_preload_main_frame) return .{};
    if (!isHttp(url)) return .{};
    const s = acquire() orelse return .{};
    defer s.release();
    if (s.siteDisabled(if (top_url.len > 0) top_url else initiator)) return .{};

    var url_buf: [4096]u8 = undefined;
    var src_buf: [4096]u8 = undefined;
    const url_z = std.fmt.bufPrintZ(&url_buf, "{s}", .{url}) catch return .{};
    const source = if (initiator.len > 0) initiator else top_url;
    const src_z = std.fmt.bufPrintZ(&src_buf, "{s}", .{source}) catch return .{};
    const kind_z = kindName(kind);

    // The user's rules first: their exceptions override the lists, and their
    // blocks apply even where the lists say nothing.
    if (s.user) |u| switch (nd_ab_check(u.raw, url_z, src_z, kind_z, null)) {
        AB_EXCEPTED => return .{},
        AB_BLOCK, AB_REDIRECT => return .{ .action = .block },
        else => {},
    };
    const e = s.lists orelse return .{};
    var redirect: ?[*:0]u8 = null;
    const verdict = nd_ab_check(e.raw, url_z, src_z, kind_z, &redirect);
    defer nd_ab_string_free(redirect);
    return switch (verdict) {
        AB_BLOCK => .{ .action = .block },
        AB_REDIRECT => redirectDecision(std.mem.span(redirect.?)),
        else => .{},
    };
}

/// A `data:<mime>;base64,<body>` URL from a `redirect=` rule, decoded for the
/// host's resource handler. Chromium refuses to follow a redirect to `data:`,
/// so the body is served in place of the original response instead.
fn redirectDecision(data_url: []const u8) Decision {
    const fallback: Decision = .{ .action = .block };
    if (!std.mem.startsWith(u8, data_url, "data:")) return fallback;
    const comma = std.mem.indexOfScalar(u8, data_url, ',') orelse return fallback;
    const meta = data_url[5..comma];
    const payload = data_url[comma + 1 ..];
    const is_b64 = std.mem.endsWith(u8, meta, ";base64");
    const mime_s = if (std.mem.indexOfScalar(u8, meta, ';')) |semi| meta[0..semi] else meta;
    const mime = gpa.dupeZ(u8, if (mime_s.len > 0) mime_s else "text/plain") catch return fallback;
    if (!is_b64) {
        const body = gpa.dupe(u8, payload) catch {
            gpa.free(mime);
            return fallback;
        };
        return .{ .action = .redirect, .body = body.ptr, .body_len = body.len, .mime = mime.ptr };
    }
    const dec = std.base64.standard.Decoder;
    const n = dec.calcSizeForSlice(payload) catch {
        gpa.free(mime);
        return fallback;
    };
    const body = gpa.alloc(u8, @max(n, 1)) catch {
        gpa.free(mime);
        return fallback;
    };
    dec.decode(body[0..n], payload) catch {
        gpa.free(body);
        gpa.free(mime);
        return fallback;
    };
    return .{ .action = .redirect, .body = body.ptr, .body_len = n, .mime = mime.ptr };
}

pub fn freeDecision(d: *Decision) void {
    if (d.body) |b| gpa.free(b[0..@max(d.body_len, 1)]);
    if (d.mime) |m| gpa.free(std.mem.span(m));
    d.* = .{};
}

// ---------------------------------------------------------------------------
// Cosmetic filtering and scriptlets, served to the renderers
// ---------------------------------------------------------------------------
//
// Every frame's main world fetches its own document-start script the moment
// its V8 context exists: both hosts' render process handlers evaluate
// `renderer_bootstrap`, whose synchronous XHR to the `nd-adblock` scheme is
// answered by `serve` on CEF's IO thread, and then evaluate what came back.
// That is ahead of the page's own scripts in every frame, cross-site ones
// included. DevTools cannot do this: a script registered while a navigation
// is in flight only reaches the documents after it.

pub const scheme = "nd-adblock";

/// The page-side half: hides what the lists name for this frame, runs the
/// procedural filters, and asks `serve` for generic class and id rules and
/// for which of its images and frames were blocked.
const cosmetic_js = @embedFile("adblock/cosmetic.js");

/// Evaluated by the render process handlers in every new main-world context;
/// yields the frame's script, or "" when there is none. The AppKit helper
/// carries its own copy (swift/Sources/CCef/nd_cef.c): it never links libnd. The XHR is captured
/// before any of the page's code exists, so the page cannot intercept it. The
/// scheme bypasses CSP, and the hosts evaluate the result directly rather
/// than through `eval`, which a page's CSP would refuse.
pub const renderer_bootstrap =
    \\(function () {
    \\  if (location.protocol !== "http:" && location.protocol !== "https:") return "";
    \\  try {
    \\    var x = new XMLHttpRequest();
    \\    x.open("GET", "nd-adblock://frame/?u=" + encodeURIComponent(location.href), false);
    \\    x.send();
    \\    return x.status === 200 ? x.responseText : "";
    \\  } catch (e) {
    \\    return "";
    \\  }
    \\})()
;

/// One frame's document-start script: the scriptlets the lists name for it,
/// then the cosmetic agent with its rules. Null when blocking is off for the
/// tab's site or nothing applies.
fn frameScript(frame_url: []const u8, top_url: []const u8) ?[]u8 {
    if (!isHttp(frame_url)) return null;
    const s = acquire() orelse return null;
    defer s.release();
    if (s.siteDisabled(if (top_url.len > 0) top_url else frame_url)) return null;
    var cosmetic = cosmeticFor(s, frame_url) orelse return null;
    defer cosmetic.deinit();
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(gpa);
    if (cosmetic.value.object.fetchOrderedRemove("injected_script")) |kv| {
        // uBO's own wrapper: its scriptlets share state through a
        // `scriptletGlobals` they expect the injector to declare, and every
        // one of them throws without it.
        if (kv.value == .string and kv.value.string.len > 0) {
            out.print(gpa, "(function () {{\nconst scriptletGlobals = {{}};\n{s}\n}})();\n", .{kv.value.string}) catch {
                out.deinit(gpa);
                return null;
            };
        }
    }
    out.print(gpa, "{s}({f});\n", .{ cosmetic_js, std.json.fmt(cosmetic.value, .{}) }) catch {
        out.deinit(gpa);
        return null;
    };
    return out.toOwnedSlice(gpa) catch null;
}

const Cosmetic = std.json.Parsed(std.json.Value);

/// adblock-rust's page resources for `url`, the lists' and the user's merged.
fn cosmeticFor(s: *const Snapshot, url: []const u8) ?Cosmetic {
    const url_z = gpa.dupeZ(u8, url) catch return null;
    defer gpa.free(url_z);
    const lists = s.lists orelse return null;
    const raw = nd_ab_url_cosmetic(lists.raw, url_z) orelse return null;
    defer nd_ab_string_free(raw);
    var parsed = std.json.parseFromSlice(std.json.Value, gpa, std.mem.span(raw), .{}) catch return null;
    if (parsed.value != .object) {
        parsed.deinit();
        return null;
    }
    if (s.user) |u| merge: {
        const user_raw = nd_ab_url_cosmetic(u.raw, url_z) orelse break :merge;
        defer nd_ab_string_free(user_raw);
        const user = std.json.parseFromSlice(std.json.Value, gpa, std.mem.span(user_raw), .{}) catch break :merge;
        defer user.deinit();
        const arena = parsed.arena.allocator();
        for ([_][]const u8{ "hide_selectors", "procedural_actions" }) |key| {
            const extra = user.value.object.get(key) orelse continue;
            const into = parsed.value.object.getPtr(key) orelse continue;
            if (extra != .array or into.* != .array) continue;
            for (extra.array.items) |item| {
                if (item != .string) continue;
                into.array.append(.{ .string = arena.dupe(u8, item.string) catch continue }) catch {};
            }
        }
    }
    return parsed;
}

/// The cosmetic agent's question about what it found in its frame: generic
/// hide selectors for the classes and ids, and which image, frame and object
/// sources the network rules block, so their boxes can be collapsed. Answers
/// `{"selectors":[..],"blocked":[..]}`.
fn genericAnswer(request_json: []const u8, top_url: []const u8) ?[]u8 {
    const Source = struct { url: []const u8 = "", kind: []const u8 = "" };
    const Req = struct {
        url: []const u8 = "",
        classes: []const []const u8 = &.{},
        ids: []const []const u8 = &.{},
        exceptions: []const []const u8 = &.{},
        sources: []const Source = &.{},
    };
    const req = std.json.parseFromSlice(Req, gpa, request_json, .{ .ignore_unknown_fields = true }) catch return null;
    defer req.deinit();
    const top = if (top_url.len > 0) top_url else req.value.url;

    var blocked: std.ArrayList([]const u8) = .empty;
    defer blocked.deinit(gpa);
    for (req.value.sources) |src| {
        const kind: ResourceType = if (std.mem.eql(u8, src.kind, "sub_frame"))
            .sub_frame
        else if (std.mem.eql(u8, src.kind, "object"))
            .object
        else if (std.mem.eql(u8, src.kind, "media"))
            .media
        else
            .image;
        var d = decide(src.url, req.value.url, top, kind);
        defer freeDecision(&d);
        if (d.action != .allow) blocked.append(gpa, src.url) catch {};
    }

    var selectors: ?[*:0]u8 = null;
    defer nd_ab_string_free(selectors);
    if (req.value.classes.len > 0 or req.value.ids.len > 0) generic: {
        const s = acquire() orelse break :generic;
        defer s.release();
        if (s.siteDisabled(top)) break :generic;
        const lists = s.lists orelse break :generic;
        const classes = std.fmt.allocPrintSentinel(gpa, "{f}", .{std.json.fmt(req.value.classes, .{})}, 0) catch break :generic;
        defer gpa.free(classes);
        const ids = std.fmt.allocPrintSentinel(gpa, "{f}", .{std.json.fmt(req.value.ids, .{})}, 0) catch break :generic;
        defer gpa.free(ids);
        const exceptions = std.fmt.allocPrintSentinel(gpa, "{f}", .{std.json.fmt(req.value.exceptions, .{})}, 0) catch break :generic;
        defer gpa.free(exceptions);
        selectors = nd_ab_hidden_selectors(lists.raw, classes, ids, exceptions);
    }
    return std.fmt.allocPrint(gpa, "{{\"selectors\":{s},\"blocked\":{f}}}", .{
        if (selectors) |p| std.mem.span(p) else "[]",
        std.json.fmt(blocked.items, .{}),
    }) catch null;
}

/// A response to one `nd-adblock:` request, which the hosts serve with
/// `Access-Control-Allow-Origin: *`. Free `body` with `freeServed`.
pub const Served = extern struct {
    body: ?[*]u8 = null,
    body_len: usize = 0,
    mime: [*:0]const u8 = "application/javascript",
};

/// Answers `nd-adblock://frame/?u=<frame url>` with that frame's script and
/// `nd-adblock://generic/?q=<json>` with `genericAnswer`. `top_url` is the
/// tab's main-frame URL, which the per-site switch keys on. Always answers:
/// an empty body means "nothing to do here". IO thread.
pub fn serve(url: []const u8, top_url: []const u8) Served {
    const query_at = std.mem.indexOf(u8, url, "/?") orelse return .{};
    const path = url[0..query_at];
    const query = url[query_at + 2 ..];
    const eq = std.mem.indexOfScalar(u8, query, '=') orelse return .{};
    const raw = gpa.dupe(u8, query[eq + 1 ..]) catch return .{};
    defer gpa.free(raw);
    const value = std.Uri.percentDecodeInPlace(raw);
    if (std.mem.endsWith(u8, path, "//frame")) {
        const body = frameScript(value, top_url) orelse return .{};
        return .{ .body = body.ptr, .body_len = body.len };
    }
    if (std.mem.endsWith(u8, path, "//generic")) {
        const body = genericAnswer(value, top_url) orelse return .{ .mime = "application/json" };
        return .{ .body = body.ptr, .body_len = body.len, .mime = "application/json" };
    }
    return .{};
}

pub fn freeServed(s: *Served) void {
    if (s.body) |b| gpa.free(b[0..s.body_len]);
    s.* = .{};
}

pub fn isSchemeUrl(url: []const u8) bool {
    return url.len > scheme.len and std.mem.startsWith(u8, url, scheme) and url[scheme.len] == ':';
}

// ---------------------------------------------------------------------------
// System methods: webviewEngine.contentBlockingLoad / contentBlockingConfigure
// ---------------------------------------------------------------------------

pub const Reply = *const fn (id: u32, ok: bool, json: []const u8) void;

pub fn handlesMethod(method: []const u8) bool {
    return std.mem.eql(u8, method, "webviewEngine.contentBlockingLoad") or
        std.mem.eql(u8, method, "webviewEngine.contentBlockingConfigure");
}

/// Runs one of the two methods. Configure answers inline; load compiles on a
/// thread of its own (the lists are ~190k lines) and answers from there.
pub fn handle(io: std.Io, id: u32, method: []const u8, params: std.json.Value, reply: Reply) void {
    if (std.mem.eql(u8, method, "webviewEngine.contentBlockingConfigure")) {
        configure(params) catch |err| {
            reply(id, false, @errorName(err));
            return;
        };
        reply(id, true, "{}");
        return;
    }
    const job = LoadJob.fromParams(params) catch |err| {
        reply(id, false, @errorName(err));
        return;
    };
    job.id = id;
    job.io = io;
    job.reply = reply;
    _ = std.Thread.spawn(.{}, LoadJob.run, .{job}) catch {
        job.deinit();
        reply(id, false, "could not start the list compiler");
    };
}

fn configure(params: std.json.Value) !void {
    if (params != .object) return error.ParamsNotAnObject;
    const next = try cloneCurrent();
    errdefer next.release();
    if (params.object.get("enabled")) |v| {
        if (v == .bool) next.enabled = v.bool;
    }
    if (params.object.get("disabledSites")) |v| if (v == .array) {
        var it = next.disabled.keyIterator();
        while (it.next()) |k| gpa.free(k.*);
        next.disabled.clearRetainingCapacity();
        for (v.array.items) |item| {
            if (item != .string or item.string.len == 0) continue;
            const host = if (std.mem.indexOf(u8, item.string, "://") != null) hostOf(item.string) else item.string;
            const gop = try next.disabled.getOrPut(gpa, host);
            if (!gop.found_existing) gop.key_ptr.* = try gpa.dupe(u8, host);
        }
    };
    if (params.object.get("userRules")) |v| if (v == .string) {
        if (next.user) |u| u.release();
        next.user = null;
        if (std.mem.trim(u8, v.string, " \r\n\t").len > 0) {
            const text = try gpa.dupeZ(u8, v.string);
            defer gpa.free(text);
            const lists = [_][*:0]const u8{text.ptr};
            const formats = [_]i32{0};
            const raw = nd_ab_engine_from_lists(&lists, &formats, 1) orelse return error.UserRulesDidNotCompile;
            next.user = Engine.create(raw) orelse return error.OutOfMemory;
        }
    };
    publish(next);
}

const LoadJob = struct {
    id: u32 = 0,
    io: std.Io = undefined,
    reply: Reply = undefined,
    lists: [][:0]u8 = &.{},
    /// How many of `lists` hold an allocation, for a parse that failed halfway.
    list_count: usize = 0,
    formats: []i32 = &.{},
    resources: ?[]u8 = null,
    cache_file: ?[]u8 = null,
    cache_key: ?[]u8 = null,

    fn fromParams(params: std.json.Value) !*LoadJob {
        if (params != .object) return error.ParamsNotAnObject;
        const lists_v = params.object.get("lists") orelse return error.MissingLists;
        if (lists_v != .array) return error.MissingLists;
        const job = try gpa.create(LoadJob);
        job.* = .{};
        errdefer job.deinit();
        job.lists = try gpa.alloc([:0]u8, lists_v.array.items.len);
        job.formats = try gpa.alloc(i32, lists_v.array.items.len);
        for (lists_v.array.items, 0..) |item, i| {
            if (item != .object) return error.BadListEntry;
            const path = item.object.get("path") orelse return error.BadListEntry;
            if (path != .string) return error.BadListEntry;
            job.lists[i] = try gpa.dupeZ(u8, path.string);
            job.list_count += 1;
            const format = item.object.get("format");
            job.formats[i] = if (format != null and format.? == .string and std.mem.eql(u8, format.?.string, "hosts")) 1 else 0;
        }
        if (params.object.get("resources")) |v| if (v == .string) {
            job.resources = try gpa.dupe(u8, v.string);
        };
        if (params.object.get("cacheFile")) |v| if (v == .string) {
            job.cache_file = try gpa.dupe(u8, v.string);
        };
        if (params.object.get("cacheKey")) |v| if (v == .string) {
            job.cache_key = try gpa.dupe(u8, v.string);
        };
        return job;
    }

    fn deinit(job: *LoadJob) void {
        for (job.lists[0..job.list_count]) |l| gpa.free(l);
        gpa.free(job.lists);
        gpa.free(job.formats);
        if (job.resources) |r| gpa.free(r);
        if (job.cache_file) |r| gpa.free(r);
        if (job.cache_key) |r| gpa.free(r);
        gpa.destroy(job);
    }

    fn run(job: *LoadJob) void {
        defer job.deinit();
        const started = std.Io.Timestamp.now(job.io, .awake);
        var from_cache = true;
        const raw = job.loadCached() orelse compiled: {
            from_cache = false;
            const r = job.compile() catch |err| {
                job.reply(job.id, false, @errorName(err));
                return;
            };
            job.writeCache(r);
            break :compiled r;
        };
        const ms = started.untilNow(job.io, .awake).toMilliseconds();
        job.finish(raw, from_cache, @intCast(@max(ms, 0)));
    }

    fn finish(job: *LoadJob, raw: *RawEngine, from_cache: bool, ms: u64) void {
        if (job.resources) |path| resources: {
            const json = std.Io.Dir.cwd().readFileAlloc(job.io, path, gpa, .limited(64 << 20)) catch |err| {
                marker.print("ND_ADBLOCK_WARN resources {s}: {s}\n", .{ path, @errorName(err) });
                break :resources;
            };
            defer gpa.free(json);
            if (!nd_ab_engine_use_resources(raw, json.ptr, json.len)) marker.print("ND_ADBLOCK_WARN resources {s}: not adblock-rust resources\n", .{path});
        }
        const engine = Engine.create(raw) orelse {
            job.reply(job.id, false, "OutOfMemory");
            return;
        };
        const next = cloneCurrent() catch {
            engine.release();
            job.reply(job.id, false, "OutOfMemory");
            return;
        };
        if (next.lists) |old| old.release();
        next.lists = engine;
        publish(next);
        marker.print("ND_ADBLOCK_LOADED lists={d} source={s} ms={d}\n", .{ job.lists.len, if (from_cache) "cache" else "compiled", ms });
        var buf: [96]u8 = undefined;
        const json = std.fmt.bufPrint(&buf, "{{\"source\":\"{s}\",\"ms\":{d}}}", .{ if (from_cache) "cache" else "compiled", ms }) catch "{}";
        job.reply(job.id, true, json);
    }

    fn compile(job: *LoadJob) !*RawEngine {
        const texts = try gpa.alloc([:0]u8, job.lists.len);
        var loaded: usize = 0;
        defer {
            for (texts[0..loaded]) |t| gpa.free(t);
            gpa.free(texts);
        }
        const ptrs = try gpa.alloc([*:0]const u8, job.lists.len);
        defer gpa.free(ptrs);
        for (job.lists, 0..) |path, i| {
            const bytes = std.Io.Dir.cwd().readFileAlloc(job.io, path, gpa, .limited(64 << 20)) catch |err| {
                marker.print("ND_ADBLOCK_WARN list {s}: {s}\n", .{ path, @errorName(err) });
                return err;
            };
            defer gpa.free(bytes);
            texts[i] = try gpa.dupeZ(u8, bytes);
            loaded += 1;
            ptrs[i] = texts[i].ptr;
        }
        return nd_ab_engine_from_lists(ptrs.ptr, job.formats.ptr, ptrs.len) orelse error.ListsDidNotCompile;
    }

    /// The cache file is the key, a newline, then adblock-rust's serialized
    /// engine. A key mismatch or bytes from another adblock-rust version both
    /// fall through to a compile.
    fn loadCached(job: *LoadJob) ?*RawEngine {
        const path = job.cache_file orelse return null;
        const key = job.cache_key orelse return null;
        const bytes = std.Io.Dir.cwd().readFileAlloc(job.io, path, gpa, .limited(256 << 20)) catch return null;
        defer gpa.free(bytes);
        const nl = std.mem.indexOfScalar(u8, bytes, '\n') orelse return null;
        if (!std.mem.eql(u8, bytes[0..nl], key)) return null;
        const body = bytes[nl + 1 ..];
        return nd_ab_engine_deserialize(body.ptr, body.len);
    }

    fn writeCache(job: *LoadJob, raw: *RawEngine) void {
        const path = job.cache_file orelse return;
        const key = job.cache_key orelse return;
        var out: [*]u8 = undefined;
        var len: usize = 0;
        if (!nd_ab_engine_serialize(raw, &out, &len)) return;
        defer nd_ab_bytes_free(out, len);
        const data = std.mem.concat(gpa, u8, &.{ key, "\n", out[0..len] }) catch return;
        defer gpa.free(data);
        std.Io.Dir.cwd().writeFile(job.io, .{ .sub_path = path, .data = data }) catch |err| {
            marker.print("ND_ADBLOCK_WARN cache {s}: {s}\n", .{ path, @errorName(err) });
        };
    }
};

// ---------------------------------------------------------------------------
// C ABI for the Swift host (the Zig engine calls the functions above)
// ---------------------------------------------------------------------------

pub export fn nd_adblock_active() callconv(.c) bool {
    snap_lock.lock();
    defer snap_lock.unlock();
    return current != null;
}

pub export fn nd_adblock_decide(url: [*:0]const u8, initiator: [*:0]const u8, top_url: [*:0]const u8, kind: c_int, out: *Decision) callconv(.c) void {
    out.* = decide(std.mem.span(url), std.mem.span(initiator), std.mem.span(top_url), @enumFromInt(kind));
}

pub export fn nd_adblock_decision_free(d: *Decision) callconv(.c) void {
    freeDecision(d);
}

pub export fn nd_adblock_serve(url: [*:0]const u8, top_url: [*:0]const u8, out: *Served) callconv(.c) void {
    out.* = serve(std.mem.span(url), std.mem.span(top_url));
}

pub export fn nd_adblock_served_free(s: *Served) callconv(.c) void {
    freeServed(s);
}

test "hostOf strips scheme, userinfo, port and path" {
    try std.testing.expectEqualStrings("www.example.com", hostOf("https://user:pw@www.example.com:8443/a?b#c"));
    try std.testing.expectEqualStrings("127.0.0.1", hostOf("http://127.0.0.1:5000"));
    try std.testing.expectEqualStrings("", hostOf("about:blank"));
    try std.testing.expectEqualStrings("[::1]", hostOf("http://[::1]:80/"));
}
