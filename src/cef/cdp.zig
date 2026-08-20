// The DevTools-protocol substrate the whole <webview> contract sits on when
// the engine is Chromium.
//
// Alloy-style CEF has no user-script API, no isolated-world API and no script
// message channel: every one of those is a CDP call. This file owns the
// mechanism (one observer per browser, one message-id space, JSON in and JSON
// out) and knows nothing about what the messages mean; engine.zig registers a
// sink and does the interpreting.
//
// Two threading rules shape the code:
//   - ExecuteDevToolsMethod only submits from the CEF UI thread, so a call made
//     from the GTK thread is posted as a cef_task_t and its parameters are
//     owned until that task runs.
//   - Results and events arrive on the CEF UI thread. Nothing here touches GTK;
//     the sink marshals.
const std = @import("std");
const capi = @import("capi.zig");
const c = capi.c;
const ref = @import("ref.zig");
const loader = @import("loader.zig");

const alloc = std.heap.c_allocator;

/// Where results and protocol events go. Both arms run on the CEF UI thread.
/// `tag` is whatever `attach` was given, opaque here.
pub const Sink = struct {
    result: *const fn (tag: usize, message_id: c_int, ok: bool, json: []const u8) void,
    event: *const fn (tag: usize, method: []const u8, json: []const u8) void,
};

var sink: ?Sink = null;

pub fn setSink(s: Sink) void {
    sink = s;
}

const ObserverObj = ref.Counted(c.cef_dev_tools_message_observer_t, usize);

/// One browser's observer registration. The registration reference IS the
/// subscription: dropping it detaches.
pub const Session = struct {
    observer: ?*ObserverObj = null,
    registration: ?*c.cef_registration_t = null,
};

/// Subscribes to one browser's protocol traffic. Safe to call from any thread:
/// add_dev_tools_message_observer carries no thread restriction.
pub fn attach(host: *c.cef_browser_host_t, tag: usize) Session {
    const add = host.add_dev_tools_message_observer orelse return .{};
    const observer = ObserverObj.create(tag) orelse return .{};
    observer.cef.on_dev_tools_method_result = &onMethodResult;
    observer.cef.on_dev_tools_event = &onEvent;
    // The registration consumes the reference it is handed, so this hands out
    // an added one and keeps ours for the teardown path.
    const registration = add(host, observer.handOut());
    if (registration == null) {
        observer.drop();
        return .{};
    }
    return .{ .observer = observer, .registration = registration };
}

pub fn detach(session: *Session) void {
    if (session.registration) |r| ref.releaseParam(r);
    session.registration = null;
    if (session.observer) |o| o.drop();
    session.observer = null;
}

fn onMethodResult(
    self: [*c]c.cef_dev_tools_message_observer_t,
    browser: [*c]c.cef_browser_t,
    message_id: c_int,
    success: c_int,
    result: ?*const anyopaque,
    result_size: usize,
) callconv(.c) void {
    defer ref.releaseParam(browser);
    const tag = ObserverObj.of(self).payload;
    const s = sink orelse return;
    const json: []const u8 = if (result) |p| @as([*]const u8, @ptrCast(p))[0..result_size] else "";
    s.result(tag, message_id, success != 0, json);
}

fn onEvent(
    self: [*c]c.cef_dev_tools_message_observer_t,
    browser: [*c]c.cef_browser_t,
    method: [*c]const c.cef_string_t,
    params: ?*const anyopaque,
    params_size: usize,
) callconv(.c) void {
    defer ref.releaseParam(browser);
    const tag = ObserverObj.of(self).payload;
    const s = sink orelse return;
    const name = utf8(method) orelse return;
    defer alloc.free(name);
    const json: []const u8 = if (params) |p| @as([*]const u8, @ptrCast(p))[0..params_size] else "";
    s.event(tag, name, json);
}

fn utf8(s: [*c]const c.cef_string_t) ?[]u8 {
    if (s == null or s.*.str == null or s.*.length == 0) return null;
    const units: []const u16 = @as([*]const u16, @ptrCast(s.*.str))[0..s.*.length];
    return std.unicode.utf16LeToUtf8Alloc(alloc, units) catch null;
}

// ============================================================================
// Sending
// ============================================================================

/// Message ids are chosen here rather than left to CEF: the return value that
/// would carry CEF's own id is only meaningful on the UI thread, and most calls
/// start on the GTK one.
var next_id: std.atomic.Value(u32) = .init(1);

pub fn nextId() c_int {
    return @intCast(next_id.fetchAdd(1, .monotonic) & 0x7fff_ffff);
}

const CallTask = struct {
    host: *c.cef_browser_host_t,
    message_id: c_int,
    method: []u8,
    params: []u8,
};

const TaskObj = ref.Counted(c.cef_task_t, CallTask);

/// Queues one CDP method call and returns the id its result will carry.
/// `params_json` is a JSON object (or empty for none) and is copied.
///
/// `host` must be a reference the caller keeps alive until the call runs; the
/// view's own long-lived host reference is what every caller passes.
pub fn send(host: *c.cef_browser_host_t, method: []const u8, params_json: []const u8) ?c_int {
    const api = loader.loaded() orelse return null;
    const id = nextId();
    const method_copy = alloc.dupe(u8, method) catch return null;
    const params_copy = alloc.dupe(u8, params_json) catch {
        alloc.free(method_copy);
        return null;
    };
    // Already on the UI thread (any handler callback): no hop, no task.
    if (api.currently_on(c.TID_UI) != 0) {
        defer alloc.free(method_copy);
        defer alloc.free(params_copy);
        execute(host, id, method_copy, params_copy);
        return id;
    }
    const task = TaskObj.create(.{
        .host = host,
        .message_id = id,
        .method = method_copy,
        .params = params_copy,
    }) orelse {
        alloc.free(method_copy);
        alloc.free(params_copy);
        return null;
    };
    task.cef.execute = &runTask;
    // post_task consumes the reference it is given.
    if (api.post_task(c.TID_UI, task.handOut()) == 0) {
        task.drop();
        return null;
    }
    task.drop();
    return id;
}

fn runTask(self: [*c]c.cef_task_t) callconv(.c) void {
    const obj = TaskObj.of(self);
    const call = obj.payload;
    defer alloc.free(call.method);
    defer alloc.free(call.params);
    execute(call.host, call.message_id, call.method, call.params);
}

fn execute(host: *c.cef_browser_host_t, id: c_int, method: []const u8, params_json: []const u8) void {
    const api = loader.loaded() orelse return;
    const run = host.execute_dev_tools_method orelse return;

    var name = std.mem.zeroes(c.cef_string_t);
    defer api.string_utf16_clear(&name);
    if (api.string_utf8_to_utf16(method.ptr, method.len, &name) == 0) return;

    // CDP parameters are a nested JSON object and cef_dictionary_value_t is a
    // key-at-a-time builder, so the parameters are written as JSON text and
    // handed to CEF's own parser rather than assembled by hand.
    var dict: [*c]c.cef_dictionary_value_t = null;
    var value: [*c]c.cef_value_t = null;
    defer if (value != null) ref.releaseParam(value);
    defer if (dict != null) ref.releaseParam(dict);
    if (params_json.len > 0) {
        value = api.parse_json_buffer(params_json.ptr, params_json.len, c.JSON_PARSER_RFC);
        if (value != null) {
            if (value.*.get_dictionary) |get| dict = get(value);
        }
        if (dict == null) {
            std.debug.print("ND_WARN CEF cdp: {s} params are not a JSON object ({s})\n", .{ method, params_json });
            return;
        }
    }
    _ = run(host, id, &name, dict);
}

// ============================================================================
// JSON helpers for building parameters and reading results
// ============================================================================

/// Appends `s` as a JSON string literal, quotes included. Written out here
/// rather than through std.json's stringifier because the callers are building
/// small parameter objects by concatenation, and the one thing that has to be
/// exactly right is escaping a script source that contains quotes and newlines.
pub fn quote(out: *std.ArrayList(u8), s: []const u8) void {
    out.append(alloc, '"') catch return;
    for (s) |ch| {
        switch (ch) {
            '"' => out.appendSlice(alloc, "\\\"") catch return,
            '\\' => out.appendSlice(alloc, "\\\\") catch return,
            '\n' => out.appendSlice(alloc, "\\n") catch return,
            '\r' => out.appendSlice(alloc, "\\r") catch return,
            '\t' => out.appendSlice(alloc, "\\t") catch return,
            0x08 => out.appendSlice(alloc, "\\b") catch return,
            0x0c => out.appendSlice(alloc, "\\f") catch return,
            // Everything else below 0x20 has no short form, and bytes at or
            // above it (UTF-8 continuation bytes included) go through as-is.
            0...0x07, 0x0b, 0x0e...0x1f => {
                var buf: [6]u8 = undefined;
                const esc = std.fmt.bufPrint(&buf, "\\u{x:0>4}", .{ch}) catch return;
                out.appendSlice(alloc, esc) catch return;
            },
            else => out.append(alloc, ch) catch return,
        }
    }
    out.append(alloc, '"') catch return;
}
