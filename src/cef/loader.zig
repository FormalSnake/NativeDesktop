// libcef.so at runtime, never at link time, on the same terms as
// gtk/webview.zig's libwebkitgtk dlopen: an app that does not ask for the
// Chromium engine never touches it, and an app that asks and does not find it
// degrades with one ND_WARN instead of failing to start.
//
// Resolution order is the spec's: ND_CEF_ROOT, then the app bundle's own
// lib/cef, then the dev cache the packaging lane populates. A "root" is the
// directory holding Release/ and Resources/; the flattened bundle layout
// (libcef.so and Resources side by side) is accepted at the same roots.
const std = @import("std");
const capi = @import("capi.zig");
const c = capi.c;

pub const version_dir = "151.3.23-linux64";

const alloc = std.heap.c_allocator;

/// Every CEF entry point this engine calls. Missing any one of them is fatal to
/// the engine (unlike the WebKit backend's per-feature degrade): none of these
/// are optional features, they are the ABI.
pub const Api = struct {
    api_hash: *const fn (version: c_int, entry: c_int) callconv(.c) [*c]const u8,
    execute_process: *const fn (args: [*c]const c.cef_main_args_t, application: [*c]c.cef_app_t, sandbox: ?*anyopaque) callconv(.c) c_int,
    initialize: *const fn (args: [*c]const c.cef_main_args_t, settings: [*c]const c.cef_settings_t, application: [*c]c.cef_app_t, sandbox: ?*anyopaque) callconv(.c) c_int,
    shutdown: *const fn () callconv(.c) void,
    create_browser: *const fn (
        window_info: [*c]const c.cef_window_info_t,
        client: [*c]c.cef_client_t,
        url: [*c]const c.cef_string_t,
        settings: [*c]const c.cef_browser_settings_t,
        extra_info: [*c]c.cef_dictionary_value_t,
        request_context: [*c]c.cef_request_context_t,
    ) callconv(.c) c_int,
    string_utf8_to_utf16: *const fn (src: [*c]const u8, src_len: usize, output: [*c]c.cef_string_utf16_t) callconv(.c) c_int,
    string_utf16_clear: *const fn (str: [*c]c.cef_string_utf16_t) callconv(.c) void,
    /// CEF's own X11 connection. Windows CEF creates are only guaranteed to
    /// exist from the connection that made them until a round trip, and an
    /// error on GDK's connection aborts the host outright, so every request
    /// against a CEF-owned window goes through this one (the same thing
    /// cefclient's GTK sample does).
    get_xdisplay: *const fn () callconv(.c) ?*anyopaque,
    /// The devtools substrate: ExecuteDevToolsMethod only submits from the CEF
    /// UI thread, so calls made from GTK are posted there first.
    currently_on: *const fn (thread_id: c.cef_thread_id_t) callconv(.c) c_int,
    post_task: *const fn (thread_id: c.cef_thread_id_t, task: [*c]c.cef_task_t) callconv(.c) c_int,
    parse_json_buffer: *const fn (json: ?*const anyopaque, size: usize, options: c.cef_json_parser_options_t) callconv(.c) [*c]c.cef_value_t,
};

var api: ?Api = null;
var attempted = false;
/// Held for the process lifetime: CEF spawns a process tree off this handle and
/// cannot be unloaded once initialized.
var lib: std.DynLib = undefined;

/// Directory holding the libcef.so that was loaded. Chromium resolves
/// icudtl.dat and the .pak files against its own module directory, so this is
/// also where the resource payload has to live (see `resourcesDir`).
var lib_dir_buf: [std.fs.max_path_bytes]u8 = undefined;
var lib_dir_len: usize = 0;

pub fn libDir() []const u8 {
    return lib_dir_buf[0..lib_dir_len];
}

/// Resolves and loads libcef.so, verifying the API pin before anything else is
/// called. Null means "no usable CEF here", and every caller treats that as a
/// fall-through to the system engine.
pub fn load() ?*const Api {
    if (attempted) return if (api != null) &api.? else null;
    attempted = true;

    var roots: std.ArrayList([]const u8) = .empty;
    defer {
        for (roots.items) |r| alloc.free(r);
        roots.deinit(alloc);
    }
    collectRoots(&roots);

    for (roots.items) |root| {
        const so = findLibrary(root) orelse continue;
        defer alloc.free(so);
        var candidate = std.DynLib.open(so) catch |err| {
            std.debug.print("ND_WARN CEF: {s} failed to load ({s})\n", .{ so, @errorName(err) });
            continue;
        };
        const resolved = lookupAll(&candidate) orelse {
            std.debug.print("ND_WARN CEF: {s} is missing entry points this engine needs\n", .{so});
            candidate.close();
            continue;
        };
        if (!checkApiHash(resolved)) {
            candidate.close();
            continue;
        }
        lib = candidate;
        api = resolved;
        const dir = std.fs.path.dirname(so) orelse root;
        lib_dir_len = @min(dir.len, lib_dir_buf.len);
        @memcpy(lib_dir_buf[0..lib_dir_len], dir[0..lib_dir_len]);
        warnSplitLayout(root);
        std.debug.print("ND_WEBVIEW_ENGINE chromium ({s})\n", .{so});
        return &api.?;
    }
    return null;
}

/// The loaded API, or null when `load` has not been called or failed. Callers
/// on the CEF UI thread use this rather than `load`, which is not reentrant.
pub fn loaded() ?*const Api {
    return if (api != null) &api.? else null;
}

fn lookupAll(l: *std.DynLib) ?Api {
    return .{
        .api_hash = l.lookup(@FieldType(Api, "api_hash"), "cef_api_hash") orelse return null,
        .execute_process = l.lookup(@FieldType(Api, "execute_process"), "cef_execute_process") orelse return null,
        .initialize = l.lookup(@FieldType(Api, "initialize"), "cef_initialize") orelse return null,
        .shutdown = l.lookup(@FieldType(Api, "shutdown"), "cef_shutdown") orelse return null,
        .create_browser = l.lookup(@FieldType(Api, "create_browser"), "cef_browser_host_create_browser") orelse return null,
        .string_utf8_to_utf16 = l.lookup(@FieldType(Api, "string_utf8_to_utf16"), "cef_string_utf8_to_utf16") orelse return null,
        .string_utf16_clear = l.lookup(@FieldType(Api, "string_utf16_clear"), "cef_string_utf16_clear") orelse return null,
        .get_xdisplay = l.lookup(@FieldType(Api, "get_xdisplay"), "cef_get_xdisplay") orelse return null,
        .currently_on = l.lookup(@FieldType(Api, "currently_on"), "cef_currently_on") orelse return null,
        .post_task = l.lookup(@FieldType(Api, "post_task"), "cef_post_task") orelse return null,
        .parse_json_buffer = l.lookup(@FieldType(Api, "parse_json_buffer"), "cef_parse_json_buffer") orelse return null,
    };
}

/// The first CEF call, before cef_execute_process and before cef_initialize.
/// A library built for a different API version has different struct layouts,
/// and every later call would write through the wrong offsets.
fn checkApiHash(a: Api) bool {
    const got = a.api_hash(capi.api_version, 0);
    if (got == null) {
        std.debug.print("ND_WARN CEF: cef_api_hash({d}) returned nothing; refusing to use this library\n", .{capi.api_version});
        return false;
    }
    // CEF_API_HASH_PLATFORM is a function-like macro translate-c cannot
    // expand, so the pinned version's own constant is selected by name.
    const want = @field(c, "CEF_API_HASH_" ++ std.fmt.comptimePrint("{d}", .{capi.api_version}));
    if (std.mem.orderZ(u8, got, want) != .eq) {
        std.debug.print("ND_WARN CEF: API hash mismatch for version {d} (library {s}, headers {s})\n", .{ capi.api_version, got, want });
        return false;
    }
    return true;
}

fn collectRoots(out: *std.ArrayList([]const u8)) void {
    if (std.c.getenv("ND_CEF_ROOT")) |env| push(out, std.mem.span(env));

    // The app bundle: `nd package` stages the dist next to the executable.
    if (selfExeDir()) |exe_dir| {
        pushJoin(out, &.{ exe_dir, "lib", "cef" });
        pushJoin(out, &.{ exe_dir, "..", "lib", "cef" });
    }

    if (std.c.getenv("HOME")) |home| {
        pushJoin(out, &.{ std.mem.span(home), ".cache", "nativedesktop", "cef", version_dir });
    }
}

var exe_dir_buf: [std.fs.max_path_bytes]u8 = undefined;

fn selfExeDir() ?[]const u8 {
    const n = std.c.readlink("/proc/self/exe", &exe_dir_buf, exe_dir_buf.len);
    if (n <= 0) return null;
    const full = exe_dir_buf[0..@intCast(n)];
    return std.fs.path.dirname(full);
}

fn push(out: *std.ArrayList([]const u8), path: []const u8) void {
    const copy = alloc.dupe(u8, path) catch return;
    out.append(alloc, copy) catch alloc.free(copy);
}

fn pushJoin(out: *std.ArrayList([]const u8), parts: []const []const u8) void {
    const joined = std.fs.path.join(alloc, parts) catch return;
    out.append(alloc, joined) catch alloc.free(joined);
}

fn exists(path: [:0]const u8) bool {
    return std.c.access(path.ptr, std.c.F_OK) == 0;
}

/// `Release/libcef.so` is the shape of an unpacked distribution; a flat
/// `libcef.so` is the shape `nd package` stages into a bundle.
fn findLibrary(root: []const u8) ?[:0]u8 {
    const layouts = [_][]const u8{ "Release/libcef.so", "libcef.so" };
    for (layouts) |rel| {
        const path = std.fmt.allocPrintSentinel(alloc, "{s}/{s}", .{ root, rel }, 0) catch continue;
        if (exists(path)) return path;
        alloc.free(path);
    }
    return null;
}

/// `resources_dir_path` for cef_settings: the directory holding icudtl.dat and
/// the .pak files, which in a working layout is the libcef.so directory itself.
///
/// The unpacked distribution splits them (`Release/` and `Resources/`), and
/// that split cannot be repaired with this setting: Chromium loads ICU inside
/// cef_initialize BEFORE CEF applies `--resources-dir-path` to DIR_ASSETS, so
/// it opens `<libcef.so dir>/icudtl.dat`, finds nothing and CHECK-fails. The
/// resource payload has to be staged next to the library, which is what
/// `nd package` produces and what `warnSplitLayout` says out loud when it is
/// not the case.
pub fn resourcesDir() ?[:0]u8 {
    const dir = libDir();
    if (dir.len == 0) return null;
    const icu = std.fmt.allocPrintSentinel(alloc, "{s}/icudtl.dat", .{dir}, 0) catch return null;
    defer alloc.free(icu);
    if (!exists(icu)) return null;
    return alloc.dupeZ(u8, dir) catch null;
}

pub fn localesDir() ?[:0]u8 {
    const res = resourcesDir() orelse return null;
    defer alloc.free(res);
    const path = std.fmt.allocPrintSentinel(alloc, "{s}/locales", .{res}, 0) catch return null;
    if (exists(path)) return path;
    alloc.free(path);
    return null;
}

/// An unpacked distribution used as-is, with its resource payload one
/// directory over from its library. cef_initialize aborts the process on this,
/// so it is worth saying before the abort rather than after.
fn warnSplitLayout(root: []const u8) void {
    const dir = libDir();
    const icu = std.fmt.allocPrintSentinel(alloc, "{s}/icudtl.dat", .{dir}, 0) catch return;
    defer alloc.free(icu);
    if (exists(icu)) return;
    std.debug.print("ND_WARN CEF: {s} has no icudtl.dat next to libcef.so; stage the dist's Resources/ into {s} (an unpacked {s} keeps them apart, and cef_initialize aborts on it)\n", .{ dir, dir, root });
}
