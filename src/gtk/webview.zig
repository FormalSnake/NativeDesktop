// GTK4 surface for the <webview> widget: a real WebKitGTK WebKitWebView when
// libwebkitgtk-6.0 is present at runtime, a placeholder GtkLabel otherwise.
// WebKitGTK is deliberately NOT a link-time dependency (a ~1GB closure,
// soname churn, no headless-CI story inside the weston harness) — the handful
// of C entry points this file needs are resolved once with std.DynLib, so the
// pinned flake and the mac GTK build (brew has no webkitgtk) stay untouched
// and degrade to the placeholder instead of failing to link.
//
// The same rule governs the browser/extension surface below (user scripts,
// script messages, custom URI schemes, cookies, find-in-page, favicons, TLS
// state, context menus, profiles, session state, audio): every symbol is
// looked up on its own and a missing one degrades ONLY its feature, with one
// ND_WARN line at load time. libsoup-3.0 (SoupCookie construction) and
// libjavascriptcoregtk (JSCValue serialization) get their own dlopens for the
// same reason.
const std = @import("std");
const gtk = @import("gtk");
const gdk = @import("gdk");
const gio = @import("gio");
const gobject = @import("gobject");
const glib = @import("glib");
const protocol = @import("../protocol.zig");
const ctxmenu = @import("context_menu.zig");

/// Peer of the generated widgets.zig EmitFn (same shape, same protocol module
/// instance) — handed over once by the generated connectEvents WebView arm.
pub const EmitFn = *const fn (node_id: u32, name: []const u8, payload: protocol.EventPayload) void;

const alloc = std.heap.page_allocator;

const MARKER_KEY = "nd-webview-real";
const NODE_ID_KEY = "nd-webview-node-id";
const PROGRESS_KEY = "nd-webview-last-progress";
const STATE_KEY = "nd-webview-state";

// WebKitLoadEvent (WebKitGTK 6.0): STARTED=0, REDIRECTED=1, COMMITTED=2, FINISHED=3.
const WEBKIT_LOAD_COMMITTED: c_int = 2;
const WEBKIT_LOAD_FINISHED: c_int = 3;
// WebKitNetworkError (WebKitGTK 6.0, webkit2/WebKitError.h) — the codes we
// filter loadFailed on, so cancelling a navigation doesn't spam apps.
const WEBKIT_NETWORK_ERROR_CANCELLED: c_int = 302;
// WebKitPolicyError.FRAME_LOAD_INTERRUPTED_BY_POLICY_CHANGE — the tail of a
// navigation cancelled by policy (e.g. a response turned into a download); it
// carries the previous page's URI, so surfacing it would fire a bogus
// loadFailed right after downloadRequested. Peer of the AppKit backend
// filtering WebKitErrorDomain 102 in NDWebView.swift.
const WEBKIT_POLICY_ERROR_FRAME_LOAD_INTERRUPTED_BY_POLICY_CHANGE: c_int = 102;

// WebKitUserContentInjectedFrames / WebKitUserScriptInjectionTime.
const INJECT_ALL_FRAMES: c_int = 0;
const INJECT_TOP_FRAME: c_int = 1;
const INJECT_AT_DOCUMENT_START: c_int = 0;
const INJECT_AT_DOCUMENT_END: c_int = 1;

// WebKitFindOptions bits.
const FIND_CASE_INSENSITIVE: u32 = 1 << 0;
const FIND_WRAP_AROUND: u32 = 1 << 4;
const FIND_MAX_MATCHES: c_uint = 1000;

// WebKitHitTestResultContext bits.
const HIT_LINK: c_uint = 1 << 2;
const HIT_IMAGE: c_uint = 1 << 3;
const HIT_MEDIA: c_uint = 1 << 4;
const HIT_EDITABLE: c_uint = 1 << 5;
const HIT_SELECTION: c_uint = 1 << 7;

// A favicon larger than this is skipped rather than pushed down the NDP pipe:
// base64 inflates by 4/3, so this caps the emitted data URL near 64 KB.
const FAVICON_MAX_PNG_BYTES: usize = 48 * 1024;

const Api = struct {
    web_view_new: *const fn () callconv(.c) *gtk.Widget,
    web_view_load_uri: *const fn (*anyopaque, [*:0]const u8) callconv(.c) void,
    web_view_get_uri: *const fn (*anyopaque) callconv(.c) ?[*:0]const u8,
    web_view_get_title: *const fn (*anyopaque) callconv(.c) ?[*:0]const u8,
    web_view_go_back: *const fn (*anyopaque) callconv(.c) void,
    web_view_go_forward: *const fn (*anyopaque) callconv(.c) void,
    web_view_reload: *const fn (*anyopaque) callconv(.c) void,
    web_view_stop_loading: *const fn (*anyopaque) callconv(.c) void,
    web_view_can_go_back: *const fn (*anyopaque) callconv(.c) c_int,
    web_view_can_go_forward: *const fn (*anyopaque) callconv(.c) c_int,
};

var api: ?Api = null;
var load_attempted = false;
// Held open for the process lifetime — WebKit spawns its own helper process
// tree and cannot be unloaded once a view exists.
var webkit_lib: std.DynLib = undefined;
var emit: ?EmitFn = null;

fn lookupAll(lib: *std.DynLib) ?Api {
    return .{
        .web_view_new = lib.lookup(@FieldType(Api, "web_view_new"), "webkit_web_view_new") orelse return null,
        .web_view_load_uri = lib.lookup(@FieldType(Api, "web_view_load_uri"), "webkit_web_view_load_uri") orelse return null,
        .web_view_get_uri = lib.lookup(@FieldType(Api, "web_view_get_uri"), "webkit_web_view_get_uri") orelse return null,
        .web_view_get_title = lib.lookup(@FieldType(Api, "web_view_get_title"), "webkit_web_view_get_title") orelse return null,
        .web_view_go_back = lib.lookup(@FieldType(Api, "web_view_go_back"), "webkit_web_view_go_back") orelse return null,
        .web_view_go_forward = lib.lookup(@FieldType(Api, "web_view_go_forward"), "webkit_web_view_go_forward") orelse return null,
        .web_view_reload = lib.lookup(@FieldType(Api, "web_view_reload"), "webkit_web_view_reload") orelse return null,
        .web_view_stop_loading = lib.lookup(@FieldType(Api, "web_view_stop_loading"), "webkit_web_view_stop_loading") orelse return null,
        .web_view_can_go_back = lib.lookup(@FieldType(Api, "web_view_can_go_back"), "webkit_web_view_can_go_back") orelse return null,
        .web_view_can_go_forward = lib.lookup(@FieldType(Api, "web_view_can_go_forward"), "webkit_web_view_can_go_forward") orelse return null,
    };
}

// ---- Browser-grade extension surface -------------------------------------
// Every symbol below is looked up independently: a missing one degrades only
// the one feature that needs it (a single ND_WARN at load time), never the
// base surface above. `jsc_value_to_string` lives in libjavascriptcoregtk,
// not libwebkitgtk, so it gets its own fallback dlopen.
const FnGetF64 = *const fn (*anyopaque) callconv(.c) f64;
const FnGetPtr = *const fn (*anyopaque) callconv(.c) ?*anyopaque;
const FnGetPtrNoArg = *const fn () callconv(.c) ?*anyopaque;
const FnGetCStr = *const fn (*anyopaque) callconv(.c) ?[*:0]const u8;
const FnGetOwnedCStr = *const fn (*anyopaque) callconv(.c) ?[*:0]u8;
const FnVoidOnPtr = *const fn (*anyopaque) callconv(.c) void;
const FnBoolOnPtr = *const fn (*anyopaque) callconv(.c) c_int;
const FnUIntOnPtr = *const fn (*anyopaque) callconv(.c) c_uint;
const FnSetF64 = *const fn (*anyopaque, f64) callconv(.c) void;
const FnSetCStrOpt = *const fn (*anyopaque, ?[*:0]const u8) callconv(.c) void;
const FnSetBool = *const fn (*anyopaque, c_int) callconv(.c) void;
const FnQuark = *const fn () callconv(.c) glib.Quark;
const FnGType = *const fn () callconv(.c) usize;
const FnEvalJsReady = *const fn (?*anyopaque, ?*anyopaque, ?*anyopaque) callconv(.c) void;
const FnEvalJs = *const fn (*anyopaque, [*:0]const u8, isize, ?[*:0]const u8, ?[*:0]const u8, ?*anyopaque, ?FnEvalJsReady, ?*anyopaque) callconv(.c) void;
const FnEvalJsFinish = *const fn (*anyopaque, ?*anyopaque, ?*?*glib.Error) callconv(.c) ?*anyopaque;
const FnJscToString = *const fn (*anyopaque) callconv(.c) ?[*:0]u8;
const FnJscToJson = *const fn (*anyopaque, c_uint) callconv(.c) ?[*:0]u8;
const FnPtrPtr = *const fn (*anyopaque, *anyopaque) callconv(.c) void;
const FnUserScriptNewForWorld = *const fn ([*:0]const u8, c_int, c_int, [*:0]const u8, ?[*]const ?[*:0]const u8, ?[*]const ?[*:0]const u8) callconv(.c) ?*anyopaque;
const FnUserScriptNew = *const fn ([*:0]const u8, c_int, c_int, ?[*]const ?[*:0]const u8, ?[*]const ?[*:0]const u8) callconv(.c) ?*anyopaque;
const FnUcmRegisterHandler = *const fn (*anyopaque, [*:0]const u8, ?[*:0]const u8) callconv(.c) c_int;
const FnUcmUnregisterHandler = *const fn (*anyopaque, [*:0]const u8, ?[*:0]const u8) callconv(.c) void;
const FnUriSchemeCallback = *const fn (?*anyopaque, ?*anyopaque) callconv(.c) void;
const FnRegisterUriScheme = *const fn (*anyopaque, [*:0]const u8, FnUriSchemeCallback, ?*anyopaque, ?*anyopaque) callconv(.c) void;
const FnSchemeFinish = *const fn (*anyopaque, *anyopaque, i64, ?[*:0]const u8) callconv(.c) void;
const FnSchemeFinishError = *const fn (*anyopaque, *glib.Error) callconv(.c) void;
const FnSchemeResponseNew = *const fn (*anyopaque, i64) callconv(.c) ?*anyopaque;
const FnSchemeResponseSetStatus = *const fn (*anyopaque, c_uint, ?[*:0]const u8) callconv(.c) void;
const FnSchemeResponseSetContentType = *const fn (*anyopaque, [*:0]const u8) callconv(.c) void;
const FnRegisterSchemeAs = *const fn (*anyopaque, [*:0]const u8) callconv(.c) void;
const FnAsyncReady = *const fn (?*anyopaque, ?*anyopaque, ?*anyopaque) callconv(.c) void;
const FnCookieGet = *const fn (*anyopaque, ?[*:0]const u8, ?*anyopaque, ?FnAsyncReady, ?*anyopaque) callconv(.c) void;
const FnCookieGetAll = *const fn (*anyopaque, ?*anyopaque, ?FnAsyncReady, ?*anyopaque) callconv(.c) void;
const FnCookieListFinish = *const fn (*anyopaque, ?*anyopaque, ?*?*glib.Error) callconv(.c) ?*glib.List;
const FnCookieMutate = *const fn (*anyopaque, *anyopaque, ?*anyopaque, ?FnAsyncReady, ?*anyopaque) callconv(.c) void;
const FnCookieMutateFinish = *const fn (*anyopaque, ?*anyopaque, ?*?*glib.Error) callconv(.c) c_int;
const FnFindSearch = *const fn (*anyopaque, [*:0]const u8, u32, c_uint) callconv(.c) void;
const FnGetTlsInfo = *const fn (*anyopaque, ?*?*anyopaque, ?*c_uint) callconv(.c) c_int;
const FnContextMenuPosition = *const fn (*anyopaque, *c_int, *c_int) callconv(.c) c_int;
const FnContextMenuItemFromAction = *const fn (*gio.Action, [*:0]const u8, ?*glib.Variant) callconv(.c) ?*anyopaque;
const FnContextMenuItemWithSubmenu = *const fn ([*:0]const u8, *anyopaque) callconv(.c) ?*anyopaque;
const FnNetworkSessionNew = *const fn (?[*:0]const u8, ?[*:0]const u8) callconv(.c) ?*anyopaque;
const FnSessionStateNew = *const fn (*glib.Bytes) callconv(.c) ?*anyopaque;
const FnSessionStateSerialize = *const fn (*anyopaque) callconv(.c) ?*glib.Bytes;

const ExtApi = struct {
    get_estimated_load_progress: ?FnGetF64 = null,
    navigation_action_get_request: ?FnGetPtr = null,
    uri_request_get_uri: ?FnGetCStr = null,
    get_network_session: ?FnGetPtr = null,
    download_get_request: ?FnGetPtr = null,
    download_get_web_view: ?FnGetPtr = null,
    download_get_response: ?FnGetPtr = null,
    uri_response_get_suggested_filename: ?FnGetCStr = null,
    download_cancel: ?FnVoidOnPtr = null,
    network_error_quark: ?FnQuark = null,
    policy_error_quark: ?FnQuark = null,
    evaluate_javascript: ?FnEvalJs = null,
    evaluate_javascript_finish: ?FnEvalJsFinish = null,
    jsc_value_to_string: ?FnJscToString = null,
    jsc_value_to_json: ?FnJscToJson = null,
    set_zoom_level: ?FnSetF64 = null,
    get_settings: ?FnGetPtr = null,
    settings_set_user_agent: ?FnSetCStrOpt = null,
    settings_set_enable_developer_extras: ?FnSetBool = null,
    get_inspector: ?FnGetPtr = null,
    web_inspector_show: ?FnVoidOnPtr = null,

    // user scripts / script messages
    get_user_content_manager: ?FnGetPtr = null,
    user_script_new: ?FnUserScriptNew = null,
    user_script_new_for_world: ?FnUserScriptNewForWorld = null,
    user_script_unref: ?FnVoidOnPtr = null,
    ucm_add_script: ?FnPtrPtr = null,
    ucm_remove_script: ?FnPtrPtr = null,
    ucm_remove_all_scripts: ?FnVoidOnPtr = null,
    ucm_register_message_handler: ?FnUcmRegisterHandler = null,
    ucm_unregister_message_handler: ?FnUcmUnregisterHandler = null,

    // custom URI schemes
    web_context_get_default: ?FnGetPtrNoArg = null,
    web_context_register_uri_scheme: ?FnRegisterUriScheme = null,
    scheme_request_get_uri: ?FnGetCStr = null,
    scheme_request_get_scheme: ?FnGetCStr = null,
    scheme_request_get_web_view: ?FnGetPtr = null,
    scheme_request_finish: ?FnSchemeFinish = null,
    scheme_request_finish_error: ?FnSchemeFinishError = null,
    scheme_request_finish_with_response: ?FnPtrPtr = null,
    scheme_response_new: ?FnSchemeResponseNew = null,
    scheme_response_set_status: ?FnSchemeResponseSetStatus = null,
    scheme_response_set_content_type: ?FnSchemeResponseSetContentType = null,
    scheme_response_set_http_headers: ?FnPtrPtr = null,
    web_context_get_security_manager: ?FnGetPtr = null,
    security_register_scheme_as_cors_enabled: ?FnRegisterSchemeAs = null,
    security_register_scheme_as_secure: ?FnRegisterSchemeAs = null,

    // cookies
    network_session_get_default: ?FnGetPtrNoArg = null,
    network_session_get_cookie_manager: ?FnGetPtr = null,
    cookie_manager_get_cookies: ?FnCookieGet = null,
    cookie_manager_get_cookies_finish: ?FnCookieListFinish = null,
    cookie_manager_get_all_cookies: ?FnCookieGetAll = null,
    cookie_manager_get_all_cookies_finish: ?FnCookieListFinish = null,
    cookie_manager_add_cookie: ?FnCookieMutate = null,
    cookie_manager_add_cookie_finish: ?FnCookieMutateFinish = null,
    cookie_manager_delete_cookie: ?FnCookieMutate = null,
    cookie_manager_delete_cookie_finish: ?FnCookieMutateFinish = null,

    // browser chrome
    get_favicon: ?FnGetPtr = null,
    network_session_get_website_data_manager: ?FnGetPtr = null,
    website_data_manager_set_favicons_enabled: ?FnSetBool = null,
    get_find_controller: ?FnGetPtr = null,
    find_search: ?FnFindSearch = null,
    find_search_next: ?FnVoidOnPtr = null,
    find_search_previous: ?FnVoidOnPtr = null,
    find_search_finish: ?FnVoidOnPtr = null,
    find_count_matches: ?FnFindSearch = null,
    get_tls_info: ?FnGetTlsInfo = null,
    hit_test_get_context: ?FnUIntOnPtr = null,
    hit_test_get_link_uri: ?FnGetCStr = null,
    hit_test_get_image_uri: ?FnGetCStr = null,
    context_menu_get_position: ?FnContextMenuPosition = null,
    context_menu_new: ?FnGetPtrNoArg = null,
    context_menu_append: ?FnPtrPtr = null,
    context_menu_item_new_from_gaction: ?FnContextMenuItemFromAction = null,
    context_menu_item_new_with_submenu: ?FnContextMenuItemWithSubmenu = null,
    context_menu_item_new_separator: ?FnGetPtrNoArg = null,

    // profiles / session state / audio
    web_view_get_type: ?FnGType = null,
    network_session_new: ?FnNetworkSessionNew = null,
    network_session_new_ephemeral: ?FnGetPtrNoArg = null,
    get_session_state: ?FnGetPtr = null,
    session_state_serialize: ?FnSessionStateSerialize = null,
    session_state_new: ?FnSessionStateNew = null,
    session_state_unref: ?FnVoidOnPtr = null,
    restore_session_state: ?FnPtrPtr = null,
    get_back_forward_list: ?FnGetPtr = null,
    bf_list_get_current_item: ?FnGetPtr = null,
    go_to_bf_list_item: ?FnPtrPtr = null,
    is_loading: ?FnBoolOnPtr = null,
    is_playing_audio: ?FnBoolOnPtr = null,
    set_is_muted: ?FnSetBool = null,
    get_is_muted: ?FnBoolOnPtr = null,
};

var ext: ExtApi = .{};
var ext_loaded = false;
// Secondary lib for jsc_value_to_string when it isn't resolvable through the
// webkitgtk handle; held open for the process lifetime like `webkit_lib`.
var jsc_lib: std.DynLib = undefined;

/// SoupCookie construction lives in libsoup-3.0, a separate shared object from
/// libwebkitgtk — same optional-degrade discipline, its own dlopen.
const SoupApi = struct {
    cookie_new: *const fn ([*:0]const u8, [*:0]const u8, [*:0]const u8, [*:0]const u8, c_int) callconv(.c) ?*anyopaque,
    cookie_free: FnVoidOnPtr,
    cookie_get_name: FnGetCStr,
    cookie_get_value: FnGetCStr,
    cookie_get_domain: FnGetCStr,
    cookie_get_path: FnGetCStr,
    cookie_get_secure: FnBoolOnPtr,
    cookie_get_http_only: FnBoolOnPtr,
    cookie_get_expires: FnGetPtr,
    cookie_get_same_site_policy: FnUIntOnPtr,
    cookie_set_secure: FnSetBool,
    cookie_set_http_only: FnSetBool,
    cookie_set_same_site_policy: *const fn (*anyopaque, c_uint) callconv(.c) void,
    /// respondScheme's `headers`. Optional on their own: an older libsoup that
    /// still has the cookie API keeps cookies working and only loses headers.
    message_headers_new: ?FnSoupHeadersNew = null,
    message_headers_append: ?FnSoupHeadersAppend = null,
};

const FnSoupHeadersNew = *const fn (c_uint) callconv(.c) ?*anyopaque;
const FnSoupHeadersAppend = *const fn (*anyopaque, [*:0]const u8, [*:0]const u8) callconv(.c) void;

/// `SOUP_MESSAGE_HEADERS_RESPONSE` (soup-message-headers.h).
const soup_headers_response: c_uint = 1;

var soup: ?SoupApi = null;
var soup_attempted = false;
var soup_lib: std.DynLib = undefined;

fn lookupWarn(comptime T: type, lib: *std.DynLib, symbol: [:0]const u8, feature: []const u8) ?T {
    return lib.lookup(T, symbol) orelse {
        std.debug.print("ND_WARN WebView {s} unavailable (missing symbol {s})\n", .{ feature, symbol });
        return null;
    };
}

fn loadJsc(lib: *std.DynLib) void {
    ext.jsc_value_to_string = lib.lookup(FnJscToString, "jsc_value_to_string");
    ext.jsc_value_to_json = lib.lookup(FnJscToJson, "jsc_value_to_json");
    if (ext.jsc_value_to_string != null and ext.jsc_value_to_json != null) return;
    const candidates = [_][]const u8{ "libjavascriptcoregtk-6.0.so.1", "libjavascriptcoregtk-6.0.so" };
    for (candidates) |name| {
        var l = std.DynLib.open(name) catch continue;
        const to_str = l.lookup(FnJscToString, "jsc_value_to_string");
        const to_json = l.lookup(FnJscToJson, "jsc_value_to_json");
        if (to_str == null and to_json == null) {
            l.close();
            continue;
        }
        jsc_lib = l;
        if (ext.jsc_value_to_string == null) ext.jsc_value_to_string = to_str;
        if (ext.jsc_value_to_json == null) ext.jsc_value_to_json = to_json;
        break;
    }
    if (ext.jsc_value_to_string == null) {
        std.debug.print("ND_WARN WebView executeJavaScript result stringification unavailable (missing symbol jsc_value_to_string)\n", .{});
    }
    if (ext.jsc_value_to_json == null) {
        std.debug.print("ND_WARN WebView scriptMessage structured bodies unavailable (missing symbol jsc_value_to_json)\n", .{});
    }
}

fn loadSoup() ?*const SoupApi {
    if (soup_attempted) return if (soup != null) &soup.? else null;
    soup_attempted = true;
    const candidates = [_][]const u8{ "libsoup-3.0.so.0", "libsoup-3.0.so" };
    for (candidates) |name| {
        var lib = std.DynLib.open(name) catch continue;
        const a: SoupApi = .{
            .cookie_new = lib.lookup(@FieldType(SoupApi, "cookie_new"), "soup_cookie_new") orelse {
                lib.close();
                continue;
            },
            .cookie_free = lib.lookup(FnVoidOnPtr, "soup_cookie_free") orelse {
                lib.close();
                continue;
            },
            .cookie_get_name = lib.lookup(FnGetCStr, "soup_cookie_get_name") orelse {
                lib.close();
                continue;
            },
            .cookie_get_value = lib.lookup(FnGetCStr, "soup_cookie_get_value") orelse {
                lib.close();
                continue;
            },
            .cookie_get_domain = lib.lookup(FnGetCStr, "soup_cookie_get_domain") orelse {
                lib.close();
                continue;
            },
            .cookie_get_path = lib.lookup(FnGetCStr, "soup_cookie_get_path") orelse {
                lib.close();
                continue;
            },
            .cookie_get_secure = lib.lookup(FnBoolOnPtr, "soup_cookie_get_secure") orelse {
                lib.close();
                continue;
            },
            .cookie_get_http_only = lib.lookup(FnBoolOnPtr, "soup_cookie_get_http_only") orelse {
                lib.close();
                continue;
            },
            .cookie_get_expires = lib.lookup(FnGetPtr, "soup_cookie_get_expires") orelse {
                lib.close();
                continue;
            },
            .cookie_get_same_site_policy = lib.lookup(FnUIntOnPtr, "soup_cookie_get_same_site_policy") orelse {
                lib.close();
                continue;
            },
            .cookie_set_secure = lib.lookup(FnSetBool, "soup_cookie_set_secure") orelse {
                lib.close();
                continue;
            },
            .cookie_set_http_only = lib.lookup(FnSetBool, "soup_cookie_set_http_only") orelse {
                lib.close();
                continue;
            },
            .cookie_set_same_site_policy = lib.lookup(@FieldType(SoupApi, "cookie_set_same_site_policy"), "soup_cookie_set_same_site_policy") orelse {
                lib.close();
                continue;
            },
        };
        soup_lib = lib;
        soup = a;
        soup.?.message_headers_new = lib.lookup(FnSoupHeadersNew, "soup_message_headers_new");
        soup.?.message_headers_append = lib.lookup(FnSoupHeadersAppend, "soup_message_headers_append");
        return &soup.?;
    }
    std.debug.print("ND_WARN WebView cookies unavailable (libsoup-3.0 not found)\n", .{});
    return null;
}

fn loadExt(lib: *std.DynLib) void {
    if (ext_loaded) return;
    ext_loaded = true;
    ext.get_estimated_load_progress = lookupWarn(FnGetF64, lib, "webkit_web_view_get_estimated_load_progress", "loadProgress");
    ext.navigation_action_get_request = lookupWarn(FnGetPtr, lib, "webkit_navigation_action_get_request", "newWindow");
    ext.uri_request_get_uri = lookupWarn(FnGetCStr, lib, "webkit_uri_request_get_uri", "newWindow/downloadRequested");
    ext.get_network_session = lookupWarn(FnGetPtr, lib, "webkit_web_view_get_network_session", "downloadRequested/cookies");
    ext.download_get_request = lookupWarn(FnGetPtr, lib, "webkit_download_get_request", "downloadRequested");
    ext.download_get_web_view = lookupWarn(FnGetPtr, lib, "webkit_download_get_web_view", "downloadRequested (per-view routing)");
    ext.download_get_response = lookupWarn(FnGetPtr, lib, "webkit_download_get_response", "downloadRequested suggestedFilename");
    ext.uri_response_get_suggested_filename = lookupWarn(FnGetCStr, lib, "webkit_uri_response_get_suggested_filename", "downloadRequested suggestedFilename");
    ext.download_cancel = lookupWarn(FnVoidOnPtr, lib, "webkit_download_cancel", "downloadRequested");
    ext.network_error_quark = lookupWarn(FnQuark, lib, "webkit_network_error_quark", "loadFailed cancellation filter");
    ext.policy_error_quark = lookupWarn(FnQuark, lib, "webkit_policy_error_quark", "loadFailed policy-interruption filter");
    ext.evaluate_javascript = lookupWarn(FnEvalJs, lib, "webkit_web_view_evaluate_javascript", "executeJavaScript");
    ext.evaluate_javascript_finish = lookupWarn(FnEvalJsFinish, lib, "webkit_web_view_evaluate_javascript_finish", "executeJavaScript");
    loadJsc(lib);
    ext.set_zoom_level = lookupWarn(FnSetF64, lib, "webkit_web_view_set_zoom_level", "setZoom");
    ext.get_settings = lookupWarn(FnGetPtr, lib, "webkit_web_view_get_settings", "setUserAgent/openDevTools");
    ext.settings_set_user_agent = lookupWarn(FnSetCStrOpt, lib, "webkit_settings_set_user_agent", "setUserAgent");
    ext.settings_set_enable_developer_extras = lookupWarn(FnSetBool, lib, "webkit_settings_set_enable_developer_extras", "openDevTools");
    ext.get_inspector = lookupWarn(FnGetPtr, lib, "webkit_web_view_get_inspector", "openDevTools");
    ext.web_inspector_show = lookupWarn(FnVoidOnPtr, lib, "webkit_web_inspector_show", "openDevTools");

    ext.get_user_content_manager = lookupWarn(FnGetPtr, lib, "webkit_web_view_get_user_content_manager", "user scripts");
    ext.user_script_new = lookupWarn(FnUserScriptNew, lib, "webkit_user_script_new", "addUserScript");
    ext.user_script_new_for_world = lookupWarn(FnUserScriptNewForWorld, lib, "webkit_user_script_new_for_world", "addUserScript (isolated worlds)");
    ext.user_script_unref = lookupWarn(FnVoidOnPtr, lib, "webkit_user_script_unref", "addUserScript");
    ext.ucm_add_script = lookupWarn(FnPtrPtr, lib, "webkit_user_content_manager_add_script", "addUserScript");
    ext.ucm_remove_script = lookupWarn(FnPtrPtr, lib, "webkit_user_content_manager_remove_script", "removeUserScript");
    ext.ucm_remove_all_scripts = lookupWarn(FnVoidOnPtr, lib, "webkit_user_content_manager_remove_all_scripts", "clearUserScripts");
    ext.ucm_register_message_handler = lookupWarn(FnUcmRegisterHandler, lib, "webkit_user_content_manager_register_script_message_handler", "registerScriptMessage");
    ext.ucm_unregister_message_handler = lookupWarn(FnUcmUnregisterHandler, lib, "webkit_user_content_manager_unregister_script_message_handler", "unregisterScriptMessage");

    ext.web_context_get_default = lookupWarn(FnGetPtrNoArg, lib, "webkit_web_context_get_default", "registerScheme");
    ext.web_context_register_uri_scheme = lookupWarn(FnRegisterUriScheme, lib, "webkit_web_context_register_uri_scheme", "registerScheme");
    ext.scheme_request_get_uri = lookupWarn(FnGetCStr, lib, "webkit_uri_scheme_request_get_uri", "registerScheme");
    ext.scheme_request_get_scheme = lookupWarn(FnGetCStr, lib, "webkit_uri_scheme_request_get_scheme", "registerScheme");
    ext.scheme_request_get_web_view = lookupWarn(FnGetPtr, lib, "webkit_uri_scheme_request_get_web_view", "registerScheme");
    ext.scheme_request_finish = lookupWarn(FnSchemeFinish, lib, "webkit_uri_scheme_request_finish", "respondScheme");
    ext.scheme_request_finish_error = lookupWarn(FnSchemeFinishError, lib, "webkit_uri_scheme_request_finish_error", "respondScheme errors");
    ext.scheme_request_finish_with_response = lookupWarn(FnPtrPtr, lib, "webkit_uri_scheme_request_finish_with_response", "respondScheme status codes");
    ext.scheme_response_new = lookupWarn(FnSchemeResponseNew, lib, "webkit_uri_scheme_response_new", "respondScheme status codes");
    ext.scheme_response_set_status = lookupWarn(FnSchemeResponseSetStatus, lib, "webkit_uri_scheme_response_set_status", "respondScheme status codes");
    ext.scheme_response_set_content_type = lookupWarn(FnSchemeResponseSetContentType, lib, "webkit_uri_scheme_response_set_content_type", "respondScheme status codes");
    ext.scheme_response_set_http_headers = lookupWarn(FnPtrPtr, lib, "webkit_uri_scheme_response_set_http_headers", "respondScheme headers");
    ext.web_context_get_security_manager = lookupWarn(FnGetPtr, lib, "webkit_web_context_get_security_manager", "registerScheme flags");
    ext.security_register_scheme_as_cors_enabled = lookupWarn(FnRegisterSchemeAs, lib, "webkit_security_manager_register_uri_scheme_as_cors_enabled", "registerScheme corsEnabled");
    ext.security_register_scheme_as_secure = lookupWarn(FnRegisterSchemeAs, lib, "webkit_security_manager_register_uri_scheme_as_secure", "registerScheme secure");

    ext.network_session_get_default = lookupWarn(FnGetPtrNoArg, lib, "webkit_network_session_get_default", "cookies");
    ext.network_session_get_cookie_manager = lookupWarn(FnGetPtr, lib, "webkit_network_session_get_cookie_manager", "cookies");
    ext.cookie_manager_get_cookies = lookupWarn(FnCookieGet, lib, "webkit_cookie_manager_get_cookies", "getCookies");
    ext.cookie_manager_get_cookies_finish = lookupWarn(FnCookieListFinish, lib, "webkit_cookie_manager_get_cookies_finish", "getCookies");
    ext.cookie_manager_get_all_cookies = lookupWarn(FnCookieGetAll, lib, "webkit_cookie_manager_get_all_cookies", "getCookies (no url)");
    ext.cookie_manager_get_all_cookies_finish = lookupWarn(FnCookieListFinish, lib, "webkit_cookie_manager_get_all_cookies_finish", "getCookies (no url)");
    ext.cookie_manager_add_cookie = lookupWarn(FnCookieMutate, lib, "webkit_cookie_manager_add_cookie", "setCookie");
    ext.cookie_manager_add_cookie_finish = lookupWarn(FnCookieMutateFinish, lib, "webkit_cookie_manager_add_cookie_finish", "setCookie");
    ext.cookie_manager_delete_cookie = lookupWarn(FnCookieMutate, lib, "webkit_cookie_manager_delete_cookie", "deleteCookie");
    ext.cookie_manager_delete_cookie_finish = lookupWarn(FnCookieMutateFinish, lib, "webkit_cookie_manager_delete_cookie_finish", "deleteCookie");

    ext.get_favicon = lookupWarn(FnGetPtr, lib, "webkit_web_view_get_favicon", "faviconChanged");
    ext.network_session_get_website_data_manager = lookupWarn(FnGetPtr, lib, "webkit_network_session_get_website_data_manager", "faviconChanged");
    ext.website_data_manager_set_favicons_enabled = lookupWarn(FnSetBool, lib, "webkit_website_data_manager_set_favicons_enabled", "faviconChanged");
    ext.get_find_controller = lookupWarn(FnGetPtr, lib, "webkit_web_view_get_find_controller", "find-in-page");
    ext.find_search = lookupWarn(FnFindSearch, lib, "webkit_find_controller_search", "findStart");
    ext.find_search_next = lookupWarn(FnVoidOnPtr, lib, "webkit_find_controller_search_next", "findNext");
    ext.find_search_previous = lookupWarn(FnVoidOnPtr, lib, "webkit_find_controller_search_previous", "findPrevious");
    ext.find_search_finish = lookupWarn(FnVoidOnPtr, lib, "webkit_find_controller_search_finish", "findStop");
    ext.find_count_matches = lookupWarn(FnFindSearch, lib, "webkit_find_controller_count_matches", "findResult match counts");
    ext.get_tls_info = lookupWarn(FnGetTlsInfo, lib, "webkit_web_view_get_tls_info", "securityChanged");
    ext.hit_test_get_context = lookupWarn(FnUIntOnPtr, lib, "webkit_hit_test_result_get_context", "linkHover/contextMenu");
    ext.hit_test_get_link_uri = lookupWarn(FnGetCStr, lib, "webkit_hit_test_result_get_link_uri", "linkHover");
    ext.hit_test_get_image_uri = lookupWarn(FnGetCStr, lib, "webkit_hit_test_result_get_image_uri", "contextMenu");
    ext.context_menu_get_position = lookupWarn(FnContextMenuPosition, lib, "webkit_context_menu_get_position", "contextMenu coordinates");
    ext.context_menu_new = lookupWarn(FnGetPtrNoArg, lib, "webkit_context_menu_new", "setContextMenuItems submenus");
    ext.context_menu_append = lookupWarn(FnPtrPtr, lib, "webkit_context_menu_append", "setContextMenuItems");
    ext.context_menu_item_new_from_gaction = lookupWarn(FnContextMenuItemFromAction, lib, "webkit_context_menu_item_new_from_gaction", "setContextMenuItems");
    ext.context_menu_item_new_with_submenu = lookupWarn(FnContextMenuItemWithSubmenu, lib, "webkit_context_menu_item_new_with_submenu", "setContextMenuItems submenus");
    ext.context_menu_item_new_separator = lookupWarn(FnGetPtrNoArg, lib, "webkit_context_menu_item_new_separator", "setContextMenuItems separators");

    ext.web_view_get_type = lookupWarn(FnGType, lib, "webkit_web_view_get_type", "profile");
    ext.network_session_new = lookupWarn(FnNetworkSessionNew, lib, "webkit_network_session_new", "profile");
    ext.network_session_new_ephemeral = lookupWarn(FnGetPtrNoArg, lib, "webkit_network_session_new_ephemeral", "profile (private)");
    ext.get_session_state = lookupWarn(FnGetPtr, lib, "webkit_web_view_get_session_state", "saveSession");
    ext.session_state_serialize = lookupWarn(FnSessionStateSerialize, lib, "webkit_web_view_session_state_serialize", "saveSession");
    ext.session_state_new = lookupWarn(FnSessionStateNew, lib, "webkit_web_view_session_state_new", "restoreSession");
    ext.session_state_unref = lookupWarn(FnVoidOnPtr, lib, "webkit_web_view_session_state_unref", "saveSession/restoreSession");
    ext.restore_session_state = lookupWarn(FnPtrPtr, lib, "webkit_web_view_restore_session_state", "restoreSession");
    ext.get_back_forward_list = lookupWarn(FnGetPtr, lib, "webkit_web_view_get_back_forward_list", "restoreSession navigation");
    ext.bf_list_get_current_item = lookupWarn(FnGetPtr, lib, "webkit_back_forward_list_get_current_item", "restoreSession navigation");
    ext.go_to_bf_list_item = lookupWarn(FnPtrPtr, lib, "webkit_web_view_go_to_back_forward_list_item", "restoreSession navigation");
    ext.is_loading = lookupWarn(FnBoolOnPtr, lib, "webkit_web_view_is_loading", "webviewInfo loading");
    ext.is_playing_audio = lookupWarn(FnBoolOnPtr, lib, "webkit_web_view_is_playing_audio", "audioStateChanged");
    ext.set_is_muted = lookupWarn(FnSetBool, lib, "webkit_web_view_set_is_muted", "setMuted");
    ext.get_is_muted = lookupWarn(FnBoolOnPtr, lib, "webkit_web_view_get_is_muted", "setMuted");
}

fn loadApi() ?*const Api {
    if (load_attempted) return if (api != null) &api.? else null;
    load_attempted = true;
    // The .4 soname is what distros actually ship; the bare .so only exists
    // with -dev packages installed. The dylib name is for a hypothetical mac
    // GTK stack that carries webkitgtk (brew's does not, today).
    const candidates = [_][]const u8{ "libwebkitgtk-6.0.so.4", "libwebkitgtk-6.0.so", "libwebkitgtk-6.0.dylib" };
    for (candidates) |name| {
        var lib = std.DynLib.open(name) catch continue;
        if (lookupAll(&lib)) |a| {
            webkit_lib = lib;
            api = a;
            loadExt(&webkit_lib);
            std.debug.print("ND_WEBVIEW_ENGINE webkitgtk ({s})\n", .{name});
            return &api.?;
        }
        lib.close();
    }
    return null;
}

// ============================================================================
// Per-view state
// ============================================================================

const ScriptEntry = struct { script: *anyopaque, world: []const u8 };

/// Everything a live WebKitWebView owns beyond the widget itself. Allocated
/// once per view at `create` and hung off the GObject under STATE_KEY (the
/// same idiom MARKER_KEY uses) so the command dispatch — which only ever
/// receives the widget — can reach it.
/// What the `contextMenuMode` prop selects. `native` keeps WebKit's own menu
/// and merges the app's items into it; `suppress` shows no engine menu at all
/// and leaves the whole decision to the app's `contextMenu` event.
const Mode = enum { native, suppress };

fn modeOf(name: []const u8) Mode {
    return if (std.mem.eql(u8, name, "suppress")) .suppress else .native;
}

const ViewState = struct {
    node_id: u32 = 0,
    mode: Mode = .native,
    /// The app's `setContextMenuItems` tree, owned by this view.
    menu_items: []ctxmenu.Item = &.{},
    scripts: std.StringHashMapUnmanaged(ScriptEntry) = .empty,
    /// Registered script-message channels, keyed by handler name — which is
    /// the whole key on this backend (see `cmdRegisterScriptMessage`).
    channels: std.StringHashMapUnmanaged(*MessageCtx) = .empty,
    last_secure: ?bool = null,
    last_playing_audio: bool = false,
};

fn stateOf(widget: *gtk.Widget) ?*ViewState {
    const raw = gobject.Object.getData(widget.as(gobject.Object), STATE_KEY) orelse return null;
    return @ptrCast(@alignCast(raw));
}

fn isReal(widget: *gtk.Widget) bool {
    return gobject.Object.getData(widget.as(gobject.Object), MARKER_KEY) != null;
}

/// backend.zig's snapshot degrade pass: a live WebKitWebView (never the
/// no-webkitgtk placeholder label) is content the snapshot renderers cannot
/// rasterize.
pub fn isRealWebView(widget: *gtk.Widget) bool {
    return isReal(widget);
}

fn widgetNodeId(widget: *gtk.Widget) ?u32 {
    const raw = gobject.Object.getData(widget.as(gobject.Object), NODE_ID_KEY) orelse return null;
    return @intCast(@intFromPtr(raw));
}

var trace_on: ?bool = null;
var trace_origin: i64 = 0;

/// ND_WEBVIEW_TRACE=1 narrates the order this view sees its own commands and
/// load transitions in. The extension surface is entirely order-dependent (a
/// user script registered after a load has begun misses document_start), and
/// this is the only place that ordering is observable.
fn tr(comptime fmt: []const u8, args: anytype) void {
    const on = trace_on orelse blk: {
        const on = std.c.getenv("ND_WEBVIEW_TRACE") != null;
        trace_on = on;
        if (on) trace_origin = glib.getMonotonicTime();
        break :blk on;
    };
    if (!on) return;
    const ms = @divTrunc(glib.getMonotonicTime() - trace_origin, 1000);
    std.debug.print("ND_WV t={d} " ++ fmt ++ "\n", .{ms} ++ args);
}

// ============================================================================
// JSON helpers
// ============================================================================

fn objPutStr(obj: *std.json.ObjectMap, key: []const u8, val: []const u8) void {
    obj.put(alloc, key, .{ .string = val }) catch {};
}

fn objPutBool(obj: *std.json.ObjectMap, key: []const u8, val: bool) void {
    obj.put(alloc, key, .{ .bool = val }) catch {};
}

fn objPutInt(obj: *std.json.ObjectMap, key: []const u8, val: i64) void {
    obj.put(alloc, key, .{ .integer = val }) catch {};
}

fn argObject(arg: ?std.json.Value) ?std.json.ObjectMap {
    return switch (arg orelse return null) {
        .object => |o| o,
        else => null,
    };
}

fn objStr(obj: std.json.ObjectMap, key: []const u8) ?[]const u8 {
    return switch (obj.get(key) orelse return null) {
        .string => |s| s,
        else => null,
    };
}

fn objBool(obj: std.json.ObjectMap, key: []const u8) ?bool {
    return switch (obj.get(key) orelse return null) {
        .bool => |b| b,
        else => null,
    };
}

fn objInt(obj: std.json.ObjectMap, key: []const u8) ?i64 {
    return switch (obj.get(key) orelse return null) {
        .integer => |i| i,
        .float => |f| @intFromFloat(f),
        else => null,
    };
}

fn objStrList(obj: std.json.ObjectMap, key: []const u8) ?std.json.Array {
    return switch (obj.get(key) orelse return null) {
        .array => |a| a,
        else => null,
    };
}

fn emitObject(node_id: u32, name: []const u8, payload: std.json.ObjectMap) void {
    const f = emit orelse return;
    f(node_id, name, .{ .data = .{ .object = payload } });
}

/// NULL-terminated `char**` for WebKit's allow/block lists. Returns null for a
/// missing or empty list, which is what WebKit wants for "no filter".
fn buildStrv(arr: ?std.json.Array) ?[]?[*:0]const u8 {
    const a = arr orelse return null;
    if (a.items.len == 0) return null;
    const out = alloc.alloc(?[*:0]const u8, a.items.len + 1) catch return null;
    var n: usize = 0;
    for (a.items) |item| {
        const s = switch (item) {
            .string => |x| x,
            else => continue,
        };
        out[n] = (alloc.dupeZ(u8, s) catch continue).ptr;
        n += 1;
    }
    out[n] = null;
    if (n == 0) {
        alloc.free(out);
        return null;
    }
    return out;
}

fn freeStrv(strv: ?[]?[*:0]const u8) void {
    const s = strv orelse return;
    for (s) |maybe| {
        const p = maybe orelse break;
        alloc.free(std.mem.span(p));
    }
    alloc.free(s);
}

// ============================================================================
// Creation
// ============================================================================

/// Named persistent profiles share one WebKitNetworkSession across every view
/// that asks for the same name (that IS what a profile means — one cookie jar,
/// one cache). Ephemeral ("private…") profiles get a fresh session per view.
var profile_sessions: std.StringHashMapUnmanaged(*anyopaque) = .empty;

fn profileSession(profile: []const u8) ?*anyopaque {
    if (std.mem.startsWith(u8, profile, "private")) {
        const mk = ext.network_session_new_ephemeral orelse {
            std.debug.print("ND_WARN WebView profile \"{s}\": ephemeral sessions unavailable, falling back to the default session\n", .{profile});
            return null;
        };
        return mk();
    }
    if (profile_sessions.get(profile)) |s| return s;
    const mk = ext.network_session_new orelse {
        std.debug.print("ND_WARN WebView profile \"{s}\": named sessions unavailable, falling back to the default session\n", .{profile});
        return null;
    };
    const base = glib.getUserDataDir();
    const data_dir = std.fmt.allocPrintSentinel(alloc, "{s}/nd-webview-profiles/{s}/data", .{ std.mem.span(base), profile }, 0) catch return null;
    defer alloc.free(data_dir);
    const cache_dir = std.fmt.allocPrintSentinel(alloc, "{s}/nd-webview-profiles/{s}/cache", .{ std.mem.span(base), profile }, 0) catch return null;
    defer alloc.free(cache_dir);
    const session = mk(data_dir.ptr, cache_dir.ptr) orelse return null;
    const key = alloc.dupe(u8, profile) catch return session;
    profile_sessions.put(alloc, key, session) catch {};
    return session;
}

/// A view bound to a non-default network session can't come from
/// `webkit_web_view_new` — "network-session" is construct-only, so the view has
/// to be built through g_object_new_with_properties.
fn createWithSession(session: *anyopaque) ?*gtk.Widget {
    const gtype_fn = ext.web_view_get_type orelse return null;
    var value: gobject.Value = std.mem.zeroes(gobject.Value);
    gobject.Value.initFromInstance(&value, @ptrCast(@alignCast(session)));
    defer gobject.Value.unset(&value);
    var names = [_][*:0]const u8{"network-session"};
    const obj = gobject.Object.newWithProperties(gtype_fn(), 1, &names, @ptrCast(&value));
    return @ptrCast(@alignCast(obj));
}

pub fn create(url: ?[*:0]const u8, profile: []const u8, context_menu_mode: []const u8) *gtk.Widget {
    const a = loadApi() orelse {
        std.debug.print("ND_WARN WebView unavailable (libwebkitgtk-6.0 not found); rendering placeholder label\n", .{});
        const label = gtk.Label.new("WebView unavailable (webkitgtk not installed)");
        return label.as(gtk.Widget);
    };
    const widget = blk: {
        if (profile.len == 0) break :blk a.web_view_new();
        const session = profileSession(profile) orelse break :blk a.web_view_new();
        break :blk createWithSession(session) orelse a.web_view_new();
    };
    gobject.Object.setData(widget.as(gobject.Object), MARKER_KEY, @ptrFromInt(1));
    const state = alloc.create(ViewState) catch {
        std.debug.print("ND_WARN WebView state allocation failed; extension surface disabled for this view\n", .{});
        return widget;
    };
    state.* = .{ .mode = modeOf(context_menu_mode) };
    gobject.Object.setData(widget.as(gobject.Object), STATE_KEY, state);
    // A webview is a content surface: expand into whatever the parent gives
    // it (a non-expanding WebKitWebView collapses to 0x0 inside a <box>).
    gtk.Widget.setHexpand(widget, 1);
    gtk.Widget.setVexpand(widget, 1);
    if (url) |u| {
        if (u[0] != 0) {
            tr("create+load url={s}", .{std.mem.span(u)});
            a.web_view_load_uri(@ptrCast(widget), u);
        }
    }
    return widget;
}

/// createAndUpdate `url` prop: navigate iff it differs from the current URI
/// (the echo guard — onNavigate feeds the URL back into app state, which
/// re-applies the prop; without the compare every navigation would reload).
pub fn setUrl(widget: *gtk.Widget, url: [:0]const u8) void {
    const a = api orelse return;
    if (!isReal(widget)) return;
    if (url.len == 0) return;
    if (a.web_view_get_uri(@ptrCast(widget))) |cur| {
        if (std.mem.eql(u8, std.mem.span(cur), url)) return;
    }
    tr("setUrl node={?d} url={s}", .{ widgetNodeId(widget), url });
    a.web_view_load_uri(@ptrCast(widget), url.ptr);
}

/// createAndUpdate `contextMenuMode` prop (generated applyProps arm).
pub fn setContextMenuMode(widget: *gtk.Widget, mode: []const u8) void {
    const state = stateOf(widget) orelse return;
    state.mode = modeOf(mode);
}

// ============================================================================
// Commands: base surface
// ============================================================================

fn cmdSetZoom(v: *anyopaque, arg: ?std.json.Value) void {
    const f = ext.set_zoom_level orelse return;
    const zoom: f64 = switch (arg orelse return) {
        .float => |x| x,
        .integer => |x| @floatFromInt(x),
        else => {
            std.debug.print("ND_WARN WebView setZoom: malformed arg (expected number)\n", .{});
            return;
        },
    };
    f(v, zoom);
}

fn cmdSetUserAgent(v: *anyopaque, arg: ?std.json.Value) void {
    const set_ua = ext.settings_set_user_agent orelse return;
    const get_settings = ext.get_settings orelse return;
    const s: []const u8 = switch (arg orelse return) {
        .string => |x| x,
        else => {
            std.debug.print("ND_WARN WebView setUserAgent: malformed arg (expected string)\n", .{});
            return;
        },
    };
    const settings = get_settings(v) orelse return;
    if (s.len == 0) {
        set_ua(settings, null); // empty string -> restore WebKit's default UA
        return;
    }
    const z = alloc.dupeZ(u8, s) catch return;
    defer alloc.free(z);
    set_ua(settings, z.ptr);
}

/// An optional world name as WebKit wants one. Absent or empty means the
/// page's own main world, which is what a NULL world_name selects, so the
/// caller frees this only when it got something back.
fn dupWorldZ(world: ?[]const u8) ?[:0]u8 {
    const name = world orelse return null;
    if (name.len == 0) return null;
    return alloc.dupeZ(u8, name) catch null;
}

fn cmdOpenDevTools(v: *anyopaque) void {
    const set_extras = ext.settings_set_enable_developer_extras orelse return;
    const get_settings = ext.get_settings orelse return;
    const get_inspector = ext.get_inspector orelse return;
    const show = ext.web_inspector_show orelse return;
    const settings = get_settings(v) orelse return;
    set_extras(settings, 1);
    const inspector = get_inspector(v) orelse return;
    show(inspector);
}

/// Async completion context for executeJavaScript: the request/response
/// pair is correlated by `id` across the NDP round trip, so it has to
/// survive past the synchronous command call — heap-allocated here, freed
/// in `cbJsEvalReady`.
const JsEvalCtx = struct {
    node_id: u32,
    id: []u8,
};

fn cmdExecuteJavaScript(widget: *gtk.Widget, v: *anyopaque, arg: ?std.json.Value) void {
    const eval = ext.evaluate_javascript orelse return;
    const obj = argObject(arg) orelse {
        std.debug.print("ND_WARN WebView executeJavaScript: malformed arg (expected {{id, code}})\n", .{});
        return;
    };
    const id = objStr(obj, "id") orelse {
        std.debug.print("ND_WARN WebView executeJavaScript: malformed arg (expected {{id, code}})\n", .{});
        return;
    };
    const code = objStr(obj, "code") orelse {
        std.debug.print("ND_WARN WebView executeJavaScript: malformed arg (expected {{id, code}})\n", .{});
        return;
    };
    const node_id = widgetNodeId(widget) orelse return;
    const ctx = alloc.create(JsEvalCtx) catch return;
    ctx.id = alloc.dupe(u8, id) catch {
        alloc.destroy(ctx);
        return;
    };
    ctx.node_id = node_id;
    const code_z = alloc.dupeZ(u8, code) catch {
        alloc.free(ctx.id);
        alloc.destroy(ctx);
        return;
    };
    defer alloc.free(code_z); // WebKit copies the script synchronously before queuing the eval
    const world_z = dupWorldZ(objStr(obj, "world"));
    defer if (world_z) |w| alloc.free(w);
    eval(v, code_z.ptr, -1, if (world_z) |w| w.ptr else null, null, null, &cbJsEvalReady, ctx);
}

fn cbJsEvalReady(source: ?*anyopaque, res: ?*anyopaque, user_data: ?*anyopaque) callconv(.c) void {
    const ctx: *JsEvalCtx = @ptrCast(@alignCast(user_data orelse return));
    defer {
        alloc.free(ctx.id);
        alloc.destroy(ctx);
    }
    const finish = ext.evaluate_javascript_finish orelse return;
    const f = emit orelse return;

    var err: ?*glib.Error = null;
    const value = finish(source orelse return, res, &err);

    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    objPutStr(&payload, "id", ctx.id);

    // The stringified result has to outlive `glib.free` — the payload only
    // borrows its strings, and they are read when `f` serializes the event.
    var value_copy: ?[]u8 = null;
    defer if (value_copy) |v| alloc.free(v);

    if (value) |jsc_value| {
        defer gobject.Object.unref(@ptrCast(@alignCast(jsc_value)));
        objPutBool(&payload, "ok", true);
        if (ext.jsc_value_to_string) |to_str| {
            if (to_str(jsc_value)) |cstr| {
                defer glib.free(cstr);
                value_copy = alloc.dupe(u8, std.mem.span(cstr)) catch null;
            }
        }
        if (value_copy) |v| objPutStr(&payload, "value", v);
        f(ctx.node_id, "javaScriptResult", .{ .data = .{ .object = payload } });
        return;
    }

    objPutBool(&payload, "ok", false);
    if (err) |e| {
        defer e.free();
        const msg: []const u8 = if (e.f_message) |m| std.mem.span(m) else "unknown error";
        objPutStr(&payload, "error", msg);
    } else {
        objPutStr(&payload, "error", "unknown error");
    }
    f(ctx.node_id, "javaScriptResult", .{ .data = .{ .object = payload } });
}

// ============================================================================
// User scripts
// ============================================================================

fn cmdAddUserScript(widget: *gtk.Widget, v: *anyopaque, arg: ?std.json.Value) void {
    const get_ucm = ext.get_user_content_manager orelse return;
    const add = ext.ucm_add_script orelse return;
    const state = stateOf(widget) orelse return;
    const obj = argObject(arg) orelse {
        std.debug.print("ND_WARN WebView addUserScript: malformed arg (expected an object)\n", .{});
        return;
    };
    const id = objStr(obj, "id") orelse {
        std.debug.print("ND_WARN WebView addUserScript: missing id\n", .{});
        return;
    };
    const source = objStr(obj, "source") orelse {
        std.debug.print("ND_WARN WebView addUserScript: missing source\n", .{});
        return;
    };
    const world = objStr(obj, "world") orelse "";
    const at_start = if (objStr(obj, "injectionTime")) |t| std.mem.eql(u8, t, "start") else false;
    const all_frames = objBool(obj, "allFrames") orelse false;

    const source_z = alloc.dupeZ(u8, source) catch return;
    defer alloc.free(source_z);
    const world_z = dupWorldZ(world);
    defer if (world_z) |w| alloc.free(w);
    const allow = buildStrv(objStrList(obj, "allowList"));
    defer freeStrv(allow);
    const block = buildStrv(objStrList(obj, "blockList"));
    defer freeStrv(block);

    const frames: c_int = if (all_frames) INJECT_ALL_FRAMES else INJECT_TOP_FRAME;
    const when: c_int = if (at_start) INJECT_AT_DOCUMENT_START else INJECT_AT_DOCUMENT_END;
    // `world_name` is NOT nullable on webkit_user_script_new_for_world — a
    // NULL there yields a script that is silently never injected. The page's
    // own world has its own constructor.
    const script = blk: {
        if (world_z) |w| {
            const mk = ext.user_script_new_for_world orelse return;
            break :blk mk(source_z.ptr, frames, when, w.ptr, if (allow) |l| l.ptr else null, if (block) |l| l.ptr else null) orelse return;
        }
        const mk = ext.user_script_new orelse return;
        break :blk mk(source_z.ptr, frames, when, if (allow) |l| l.ptr else null, if (block) |l| l.ptr else null) orelse return;
    };
    const ucm = get_ucm(v) orelse {
        if (ext.user_script_unref) |unref| unref(script);
        return;
    };
    // Re-adding under a live id replaces it, matching how apps think about a
    // keyed registry (and how the AppKit side has to behave anyway).
    removeScriptById(v, state, id);
    tr("addUserScript node={?d} id={s} world={s} at={s}", .{ widgetNodeId(widget), id, world, if (at_start) "start" else "end" });
    add(ucm, script);
    const key = alloc.dupe(u8, id) catch return;
    const world_copy = alloc.dupe(u8, world) catch "";
    state.scripts.put(alloc, key, .{ .script = script, .world = world_copy }) catch {};
}

fn removeScriptById(v: *anyopaque, state: *ViewState, id: []const u8) void {
    const entry = state.scripts.fetchRemove(id) orelse return;
    defer alloc.free(entry.key);
    defer if (entry.value.world.len > 0) alloc.free(entry.value.world);
    if (ext.get_user_content_manager) |get_ucm| {
        if (ext.ucm_remove_script) |remove| {
            if (get_ucm(v)) |ucm| remove(ucm, entry.value.script);
        }
    }
    if (ext.user_script_unref) |unref| unref(entry.value.script);
}

fn cmdRemoveUserScript(widget: *gtk.Widget, v: *anyopaque, arg: ?std.json.Value) void {
    const state = stateOf(widget) orelse return;
    const obj = argObject(arg) orelse return;
    const id = objStr(obj, "id") orelse {
        std.debug.print("ND_WARN WebView removeUserScript: missing id\n", .{});
        return;
    };
    removeScriptById(v, state, id);
}

fn cmdClearUserScripts(widget: *gtk.Widget, v: *anyopaque, arg: ?std.json.Value) void {
    const state = stateOf(widget) orelse return;
    tr("clearUserScripts node={?d}", .{widgetNodeId(widget)});
    const world: ?[]const u8 = if (argObject(arg)) |o| objStr(o, "world") else null;
    if (world) |w| {
        // World-scoped clear has no WebKit equivalent, so walk the registry
        // and remove by identity. Collecting ids first keeps the iterator
        // valid across the removals.
        var ids: std.ArrayList([]const u8) = .empty;
        defer ids.deinit(alloc);
        var it = state.scripts.iterator();
        while (it.next()) |e| {
            if (std.mem.eql(u8, e.value_ptr.world, w)) ids.append(alloc, e.key_ptr.*) catch {};
        }
        for (ids.items) |id| removeScriptById(v, state, id);
        return;
    }
    if (ext.get_user_content_manager) |get_ucm| {
        if (ext.ucm_remove_all_scripts) |remove_all| {
            if (get_ucm(v)) |ucm| remove_all(ucm);
        }
    }
    var it = state.scripts.iterator();
    while (it.next()) |e| {
        if (ext.user_script_unref) |unref| unref(e.value_ptr.script);
        alloc.free(e.key_ptr.*);
        if (e.value_ptr.world.len > 0) alloc.free(e.value_ptr.world);
    }
    state.scripts.clearRetainingCapacity();
}

// ============================================================================
// Script messages
// ============================================================================

/// One per registered channel — the signal callback gets no node id of its
/// own, and the detail-qualified signal carries only the posted JSCValue.
const MessageCtx = struct {
    node_id: u32,
    name: []u8,
    world: []u8,
    handler_id: c_ulong = 0,
};

fn destroyMessageCtx(ctx: *MessageCtx) void {
    alloc.free(ctx.name);
    if (ctx.world.len > 0) alloc.free(ctx.world);
    alloc.destroy(ctx);
}

/// A handler NAME is the whole key on this backend, in both directions: the
/// signal detail is the name, and WebKitGTK refuses a name already registered
/// on the manager whatever world is asked for. So a repeat is one of two
/// things, and neither may connect a second callback to the same detail —
/// that would deliver every message twice:
///
///   * the same channel re-armed (the app rebuilding a view's script set),
///     which is a no-op;
///   * a second world asking for a name it cannot have, which is a caller bug
///     and is named as one, because the messages it silently mis-delivers are
///     the other world's.
fn cmdRegisterScriptMessage(widget: *gtk.Widget, v: *anyopaque, arg: ?std.json.Value) void {
    const get_ucm = ext.get_user_content_manager orelse return;
    const register = ext.ucm_register_message_handler orelse return;
    const obj = argObject(arg) orelse return;
    const name = objStr(obj, "name") orelse {
        std.debug.print("ND_WARN WebView registerScriptMessage: missing name\n", .{});
        return;
    };
    const world = objStr(obj, "world") orelse "";
    const node_id = widgetNodeId(widget) orelse return;
    const state = stateOf(widget) orelse return;
    const ucm = get_ucm(v) orelse return;

    if (state.channels.get(name)) |existing| {
        if (!std.mem.eql(u8, existing.world, world)) {
            std.debug.print(
                "ND_WARN WebView registerScriptMessage: '{s}' is already registered on this view in world '{s}'; a handler name is per view, not per world — give world '{s}' a name of its own\n",
                .{ name, existing.world, world },
            );
        }
        return;
    }

    const name_z = alloc.dupeZ(u8, name) catch return;
    defer alloc.free(name_z);
    const world_z = dupWorldZ(world);
    defer if (world_z) |w| alloc.free(w);
    const detail = std.fmt.allocPrintSentinel(alloc, "script-message-received::{s}", .{name}, 0) catch return;
    defer alloc.free(detail);

    tr("registerScriptMessage node={d} name={s} world={s}", .{ node_id, name, world });
    const ctx = alloc.create(MessageCtx) catch return;
    ctx.* = .{
        .node_id = node_id,
        .name = alloc.dupe(u8, name) catch {
            alloc.destroy(ctx);
            return;
        },
        .world = alloc.dupe(u8, world) catch "",
    };
    // Connect BEFORE registering: WebKit's own docs call out the race where a
    // message posted between the two calls would be dropped.
    ctx.handler_id = gobject.signalConnectData(@ptrCast(@alignCast(ucm)), detail, @ptrCast(&cbScriptMessage), ctx, null, .{});
    if (register(ucm, name_z.ptr, if (world_z) |w| w.ptr else null) == 0) {
        gobject.signalHandlerDisconnect(@ptrCast(@alignCast(ucm)), ctx.handler_id);
        destroyMessageCtx(ctx);
        std.debug.print("ND_WARN WebView registerScriptMessage: WebKitGTK refused the name '{s}'\n", .{name});
        return;
    }
    state.channels.put(alloc, ctx.name, ctx) catch {};
}

fn cmdUnregisterScriptMessage(widget: *gtk.Widget, v: *anyopaque, arg: ?std.json.Value) void {
    const get_ucm = ext.get_user_content_manager orelse return;
    const unregister = ext.ucm_unregister_message_handler orelse return;
    const obj = argObject(arg) orelse return;
    const name = objStr(obj, "name") orelse return;
    const state = stateOf(widget) orelse return;
    const ucm = get_ucm(v) orelse return;
    const entry = state.channels.fetchRemove(name) orelse return;
    const ctx = entry.value;
    // The world comes off the registration rather than the argument: this
    // backend keys on the name, so the pair that went in is the pair that has
    // to come out.
    if (alloc.dupeZ(u8, ctx.name)) |name_z| {
        defer alloc.free(name_z);
        const world_z = dupWorldZ(ctx.world);
        defer if (world_z) |w| alloc.free(w);
        unregister(ucm, name_z.ptr, if (world_z) |w| w.ptr else null);
    } else |_| {}
    // Unregistering leaves the signal connected (WebKit's docs are explicit),
    // so a stale callback would fire again the moment the name is re-used.
    gobject.signalHandlerDisconnect(@ptrCast(@alignCast(ucm)), ctx.handler_id);
    destroyMessageCtx(ctx);
}

fn cbScriptMessage(_: *gobject.Object, value: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    const ctx: *MessageCtx = @ptrCast(@alignCast(data orelse return));
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    objPutStr(&payload, "name", ctx.name);
    objPutStr(&payload, "world", ctx.world);

    // `body` is the posted value itself, decoded: jsc_value_to_json gives the
    // exact JSON the page sent, which is parsed back into a real JSON value so
    // apps read `e.data.body.foo` instead of re-parsing a string. Values JSON
    // can't express (functions, undefined) fall back to the string form.
    var parsed: ?std.json.Parsed(std.json.Value) = null;
    defer if (parsed) |p| p.deinit();
    // Same rule as the executeJavaScript path: the payload borrows, so the
    // string form must outlive `glib.free` and the emit below.
    var body_copy: ?[]u8 = null;
    defer if (body_copy) |b| alloc.free(b);
    var body_set = false;
    if (value) |jsc| {
        if (ext.jsc_value_to_json) |to_json| {
            if (to_json(jsc, 0)) |cstr| {
                defer glib.free(cstr);
                if (std.json.parseFromSlice(std.json.Value, alloc, std.mem.span(cstr), .{})) |p| {
                    parsed = p;
                    payload.put(alloc, "body", p.value) catch {};
                    body_set = true;
                } else |_| {}
            }
        }
        if (!body_set) {
            if (ext.jsc_value_to_string) |to_str| {
                if (to_str(jsc)) |cstr| {
                    defer glib.free(cstr);
                    body_copy = alloc.dupe(u8, std.mem.span(cstr)) catch null;
                }
            }
            if (body_copy) |b| {
                objPutStr(&payload, "body", b);
                body_set = true;
            }
        }
    }
    if (!body_set) payload.put(alloc, "body", .null) catch {};
    emitObject(ctx.node_id, "scriptMessage", payload);
}

// ============================================================================
// Custom URI schemes
// ============================================================================

var registered_schemes: std.StringHashMapUnmanaged(void) = .empty;
var pending_scheme_requests: std.StringHashMapUnmanaged(*anyopaque) = .empty;
var scheme_seq: u64 = 0;
/// Set once a webview exists; scheme registration after that point is too late
/// on AppKit (the handler must be on the configuration before view creation),
/// so both backends report the same error rather than silently differing.
var any_view_created = false;

pub const RegisterSchemeError = error{
    Unsupported,
    TooLate,
    AlreadyRegistered,
    Invalid,
};

/// `webviewEngine.registerScheme` (systemRequest, ACL `core:webview`). GTK
/// registers on the DEFAULT WebKitWebContext, which every `<webview>` shares,
/// so this is process-wide rather than per-view.
///
/// `cors_enabled` and `secure` go through the context's WebKitSecurityManager:
/// without the first, a page cannot `fetch()` the scheme cross-origin at all
/// (no response header can grant what the security origin refuses); without
/// the second the origin is not a secure context, so `crypto.subtle`,
/// IndexedDB and service workers are unavailable to it.
pub fn registerScheme(scheme: []const u8, cors_enabled: bool, secure: bool) RegisterSchemeError!void {
    if (scheme.len == 0) return error.Invalid;
    for (scheme) |c| {
        const ok = (c >= 'a' and c <= 'z') or (c >= '0' and c <= '9') or c == '+' or c == '-' or c == '.';
        if (!ok) return error.Invalid;
    }
    if (registered_schemes.contains(scheme)) return error.AlreadyRegistered;
    if (any_view_created) return error.TooLate;
    _ = loadApi() orelse return error.Unsupported;
    const get_ctx = ext.web_context_get_default orelse return error.Unsupported;
    const register = ext.web_context_register_uri_scheme orelse return error.Unsupported;
    const ctx = get_ctx() orelse return error.Unsupported;
    const scheme_z = alloc.dupeZ(u8, scheme) catch return error.Unsupported;
    register(ctx, scheme_z.ptr, &cbSchemeRequest, null, null);
    if (cors_enabled or secure) {
        if (ext.web_context_get_security_manager) |get_sm| {
            if (get_sm(ctx)) |sm| {
                if (cors_enabled) {
                    if (ext.security_register_scheme_as_cors_enabled) |f| f(sm, scheme_z.ptr);
                }
                if (secure) {
                    if (ext.security_register_scheme_as_secure) |f| f(sm, scheme_z.ptr);
                }
            }
        }
    }
    const key = alloc.dupe(u8, scheme) catch return;
    registered_schemes.put(alloc, key, {}) catch {};
}

fn cbSchemeRequest(request: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
    const req = request orelse return;
    const get_uri = ext.scheme_request_get_uri orelse return;
    const get_scheme = ext.scheme_request_get_scheme orelse return;
    const get_view = ext.scheme_request_get_web_view orelse return;

    const view = get_view(req) orelse return schemeFail(req, "no web view for request");
    const node_id = widgetNodeId(@ptrCast(@alignCast(view))) orelse
        return schemeFail(req, "web view is not connected to a node");

    scheme_seq += 1;
    const id = std.fmt.allocPrint(alloc, "sch{d}", .{scheme_seq}) catch return schemeFail(req, "oom");
    _ = gobject.Object.ref(@ptrCast(@alignCast(req)));
    pending_scheme_requests.put(alloc, id, req) catch {
        gobject.Object.unref(@ptrCast(@alignCast(req)));
        alloc.free(id);
        return schemeFail(req, "oom");
    };

    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    objPutStr(&payload, "id", id);
    objPutStr(&payload, "url", if (get_uri(req)) |u| std.mem.span(u) else "");
    objPutStr(&payload, "scheme", if (get_scheme(req)) |s| std.mem.span(s) else "");
    emitObject(node_id, "schemeRequest", payload);
}

fn schemeFail(req: *anyopaque, message: [:0]const u8) void {
    const finish_error = ext.scheme_request_finish_error orelse return;
    const err = glib.Error.newLiteral(glib.quarkFromString("nd-webview"), 1, message);
    defer err.free();
    finish_error(req, err);
}

fn cmdRespondScheme(arg: ?std.json.Value) void {
    const obj = argObject(arg) orelse return;
    const id = objStr(obj, "id") orelse {
        std.debug.print("ND_WARN WebView respondScheme: missing id\n", .{});
        return;
    };
    const entry = pending_scheme_requests.fetchRemove(id) orelse {
        std.debug.print("ND_WARN WebView respondScheme: unknown request id {s}\n", .{id});
        return;
    };
    defer alloc.free(entry.key);
    const req = entry.value;
    defer gobject.Object.unref(@ptrCast(@alignCast(req)));

    if (objStr(obj, "error")) |msg| {
        const z = alloc.dupeZ(u8, msg) catch "scheme request failed";
        defer if (z.len > 0) alloc.free(@constCast(z));
        schemeFail(req, z);
        return;
    }

    const b64 = objStr(obj, "base64") orelse {
        schemeFail(req, "respondScheme: missing base64 body");
        return;
    };
    const decoder = std.base64.standard.Decoder;
    const size = decoder.calcSizeForSlice(b64) catch {
        schemeFail(req, "respondScheme: malformed base64 body");
        return;
    };
    // Owned by the GMemoryInputStream (freed through g_free), so it must come
    // from GLib's allocator, not Zig's.
    const raw = glib.malloc(@max(size, 1)) orelse {
        schemeFail(req, "respondScheme: out of memory");
        return;
    };
    const buf: []u8 = @as([*]u8, @ptrCast(raw))[0..size];
    decoder.decode(buf, b64) catch {
        glib.free(raw);
        schemeFail(req, "respondScheme: malformed base64 body");
        return;
    };
    const stream = gio.MemoryInputStream.newFromData(@ptrCast(raw), @intCast(size), @ptrCast(&glib.free));
    defer gobject.Object.unref(stream.as(gobject.Object));

    const mime = objStr(obj, "mime") orelse "application/octet-stream";
    const mime_z = alloc.dupeZ(u8, mime) catch {
        schemeFail(req, "respondScheme: out of memory");
        return;
    };
    defer alloc.free(mime_z);

    // The response object carries the HTTP status and any response headers;
    // without it (older WebKitGTK) the plain finish still serves the bytes,
    // just always as 200 and headerless.
    const status = objInt(obj, "status");
    const headers: ?std.json.ObjectMap = if (obj.get("headers")) |h| (if (h == .object) h.object else null) else null;
    if (status != null or headers != null) {
        if (ext.scheme_response_new) |resp_new| {
            if (ext.scheme_response_set_status) |set_status| {
                if (ext.scheme_response_set_content_type) |set_type| {
                    if (ext.scheme_request_finish_with_response) |finish_resp| {
                        if (resp_new(@ptrCast(stream), @intCast(size))) |resp| {
                            defer gobject.Object.unref(@ptrCast(@alignCast(resp)));
                            set_status(resp, @intCast(status orelse 200), null);
                            set_type(resp, mime_z.ptr);
                            if (headers) |h| applySchemeHeaders(resp, h);
                            finish_resp(req, resp);
                            return;
                        }
                    }
                }
            }
        }
        if (headers != null) {
            std.debug.print("ND_WARN WebView respondScheme: response headers dropped (webkitgtk too old)\n", .{});
        }
    }
    const finish = ext.scheme_request_finish orelse return;
    finish(req, @ptrCast(stream), @intCast(size), mime_z.ptr);
}

/// `{"Content-Security-Policy": "…", …}` onto the response. The SoupMessageHeaders
/// is transfer-full into `webkit_uri_scheme_response_set_http_headers`, so it is
/// deliberately not freed here.
fn applySchemeHeaders(resp: *anyopaque, headers: std.json.ObjectMap) void {
    const set_headers = ext.scheme_response_set_http_headers orelse {
        std.debug.print("ND_WARN WebView respondScheme: response headers dropped (webkitgtk too old)\n", .{});
        return;
    };
    const s = loadSoup() orelse return;
    const new_headers = s.message_headers_new orelse return;
    const append = s.message_headers_append orelse return;
    const hdrs = new_headers(soup_headers_response) orelse return;
    var it = headers.iterator();
    while (it.next()) |entry| {
        if (entry.value_ptr.* != .string) continue;
        const name = alloc.dupeZ(u8, entry.key_ptr.*) catch continue;
        defer alloc.free(name);
        const value = alloc.dupeZ(u8, entry.value_ptr.string) catch continue;
        defer alloc.free(value);
        append(hdrs, name.ptr, value.ptr);
    }
    set_headers(resp, hdrs);
}

// ============================================================================
// Cookies
// ============================================================================

const CookieCtx = struct {
    node_id: u32,
    id: []u8,
};

fn cookieManager(v: *anyopaque) ?*anyopaque {
    const get_cm = ext.network_session_get_cookie_manager orelse return null;
    // The view's OWN session, so a `profile` view reads its own jar; falling
    // back to the process default keeps pre-profile behaviour intact.
    if (ext.get_network_session) |get_session| {
        if (get_session(v)) |session| return get_cm(session);
    }
    const get_default = ext.network_session_get_default orelse return null;
    const session = get_default() orelse return null;
    return get_cm(session);
}

fn cmdGetCookies(widget: *gtk.Widget, v: *anyopaque, arg: ?std.json.Value) void {
    const obj = argObject(arg) orelse return;
    const id = objStr(obj, "id") orelse {
        std.debug.print("ND_WARN WebView getCookies: missing id\n", .{});
        return;
    };
    const node_id = widgetNodeId(widget) orelse return;
    const cm = cookieManager(v) orelse return cookiesError(node_id, id, "cookie manager unavailable");
    if (loadSoup() == null) return cookiesError(node_id, id, "libsoup-3.0 unavailable");

    const ctx = alloc.create(CookieCtx) catch return;
    ctx.* = .{ .node_id = node_id, .id = alloc.dupe(u8, id) catch {
        alloc.destroy(ctx);
        return;
    } };

    if (objStr(obj, "url")) |url| {
        const get = ext.cookie_manager_get_cookies orelse return cookiesError(node_id, id, "getCookies unavailable");
        const url_z = alloc.dupeZ(u8, url) catch return;
        defer alloc.free(url_z);
        get(cm, url_z.ptr, null, &cbCookiesForUrl, ctx);
        return;
    }
    const get_all = ext.cookie_manager_get_all_cookies orelse return cookiesError(node_id, id, "getCookies (no url) unavailable");
    get_all(cm, null, &cbCookiesAll, ctx);
}

fn cookiesError(node_id: u32, id: []const u8, message: []const u8) void {
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    objPutStr(&payload, "id", id);
    objPutBool(&payload, "ok", false);
    objPutStr(&payload, "error", message);
    emitObject(node_id, "cookiesResult", payload);
}

fn cbCookiesForUrl(source: ?*anyopaque, res: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    finishCookies(ext.cookie_manager_get_cookies_finish, source, res, data);
}

fn cbCookiesAll(source: ?*anyopaque, res: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    finishCookies(ext.cookie_manager_get_all_cookies_finish, source, res, data);
}

fn finishCookies(finish_fn: ?FnCookieListFinish, source: ?*anyopaque, res: ?*anyopaque, data: ?*anyopaque) void {
    const ctx: *CookieCtx = @ptrCast(@alignCast(data orelse return));
    defer {
        alloc.free(ctx.id);
        alloc.destroy(ctx);
    }
    const finish = finish_fn orelse return cookiesError(ctx.node_id, ctx.id, "getCookies unavailable");
    const s = loadSoup() orelse return cookiesError(ctx.node_id, ctx.id, "libsoup-3.0 unavailable");

    var err: ?*glib.Error = null;
    const list = finish(source orelse return, res, &err);
    if (list == null) {
        // A NULL list with no GError is an EMPTY jar, not a failure — WebKit
        // returns NULL for both, and only the error tells them apart.
        if (err) |e| {
            defer e.free();
            const msg: []const u8 = if (e.f_message) |m| std.mem.span(m) else "getCookies failed";
            cookiesError(ctx.node_id, ctx.id, msg);
            return;
        }
        var empty: std.json.ObjectMap = .empty;
        defer empty.deinit(alloc);
        objPutStr(&empty, "id", ctx.id);
        objPutBool(&empty, "ok", true);
        var none: std.json.Array = .init(alloc);
        defer none.deinit();
        empty.put(alloc, "cookies", .{ .array = none }) catch {};
        emitObject(ctx.node_id, "cookiesResult", empty);
        return;
    }

    // Every string handed to the JSON payload is borrowed from the SoupCookie
    // list, which stays alive until the free below — the emit is synchronous,
    // so no copies are needed.
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    objPutStr(&payload, "id", ctx.id);
    objPutBool(&payload, "ok", true);
    var cookies: std.json.Array = .init(alloc);
    defer {
        for (cookies.items) |item| {
            switch (item) {
                .object => |o| {
                    var m = o;
                    m.deinit(alloc);
                },
                else => {},
            }
        }
        cookies.deinit();
    }

    var node = list;
    while (node) |n| : (node = n.f_next) {
        const raw = n.f_data orelse continue;
        var c: std.json.ObjectMap = .empty;
        objPutStr(&c, "name", if (s.cookie_get_name(raw)) |x| std.mem.span(x) else "");
        objPutStr(&c, "value", if (s.cookie_get_value(raw)) |x| std.mem.span(x) else "");
        objPutStr(&c, "domain", if (s.cookie_get_domain(raw)) |x| std.mem.span(x) else "");
        objPutStr(&c, "path", if (s.cookie_get_path(raw)) |x| std.mem.span(x) else "");
        objPutBool(&c, "secure", s.cookie_get_secure(raw) != 0);
        objPutBool(&c, "httpOnly", s.cookie_get_http_only(raw) != 0);
        if (s.cookie_get_expires(raw)) |dt| {
            objPutInt(&c, "expires", glib.DateTime.toUnix(@ptrCast(@alignCast(dt))));
        } else {
            c.put(alloc, "expires", .null) catch {};
        }
        objPutStr(&c, "sameSite", switch (s.cookie_get_same_site_policy(raw)) {
            1 => "Lax",
            2 => "Strict",
            else => "None",
        });
        cookies.append(.{ .object = c }) catch {};
    }
    payload.put(alloc, "cookies", .{ .array = cookies }) catch {};
    emitObject(ctx.node_id, "cookiesResult", payload);

    glib.List.freeFull(list.?, @ptrCast(s.cookie_free));
}

/// Builds a SoupCookie from the JSON shape both commands accept. Callers own
/// the result and must free it with `soup_cookie_free`.
fn buildCookie(s: *const SoupApi, obj: std.json.ObjectMap, default_value: []const u8) ?*anyopaque {
    const name = objStr(obj, "name") orelse return null;
    const value = objStr(obj, "value") orelse default_value;
    const domain = objStr(obj, "domain") orelse return null;
    const path = objStr(obj, "path") orelse "/";
    const name_z = alloc.dupeZ(u8, name) catch return null;
    defer alloc.free(name_z);
    const value_z = alloc.dupeZ(u8, value) catch return null;
    defer alloc.free(value_z);
    const domain_z = alloc.dupeZ(u8, domain) catch return null;
    defer alloc.free(domain_z);
    const path_z = alloc.dupeZ(u8, path) catch return null;
    defer alloc.free(path_z);
    // SoupCookie takes a max-age, not an absolute time; -1 means a session
    // cookie, which is the right default when the app gives no expiry.
    const max_age: c_int = blk: {
        const expires = objInt(obj, "expires") orelse break :blk -1;
        const now = @divTrunc(glib.getRealTime(), std.time.us_per_s);
        const delta = expires - now;
        break :blk if (delta <= 0) 0 else @intCast(@min(delta, std.math.maxInt(c_int)));
    };
    const cookie = s.cookie_new(name_z.ptr, value_z.ptr, domain_z.ptr, path_z.ptr, max_age) orelse return null;
    if (objBool(obj, "secure")) |b| s.cookie_set_secure(cookie, if (b) 1 else 0);
    if (objBool(obj, "httpOnly")) |b| s.cookie_set_http_only(cookie, if (b) 1 else 0);
    if (objStr(obj, "sameSite")) |policy| {
        const p: c_uint = if (std.ascii.eqlIgnoreCase(policy, "lax"))
            1
        else if (std.ascii.eqlIgnoreCase(policy, "strict"))
            2
        else
            0;
        s.cookie_set_same_site_policy(cookie, p);
    }
    return cookie;
}

fn cookieArg(arg: ?std.json.Value) ?std.json.ObjectMap {
    const obj = argObject(arg) orelse return null;
    // Both `{cookie: {...}}` and the bare cookie object are accepted.
    if (obj.get("cookie")) |nested| {
        return switch (nested) {
            .object => |o| o,
            else => obj,
        };
    }
    return obj;
}

fn cmdSetCookie(v: *anyopaque, arg: ?std.json.Value) void {
    const s = loadSoup() orelse return;
    const add = ext.cookie_manager_add_cookie orelse return;
    const cm = cookieManager(v) orelse return;
    const obj = cookieArg(arg) orelse return;
    const cookie = buildCookie(s, obj, "") orelse {
        std.debug.print("ND_WARN WebView setCookie: malformed cookie (name and domain are required)\n", .{});
        return;
    };
    defer s.cookie_free(cookie);
    add(cm, cookie, null, &cbCookieAdded, null);
}

/// libsoup's cookie jar deletes by full identity — name, VALUE and path all
/// have to match — so a cookie synthesized from {name, domain, path} never
/// matches the stored one. Read the jar first and delete the live cookies,
/// which is also exactly what the AppKit peer does with WKHTTPCookieStore.
const DeleteCookieCtx = struct {
    name: []u8,
    domain: ?[]u8,
    path: ?[]u8,
};

fn cmdDeleteCookie(v: *anyopaque, arg: ?std.json.Value) void {
    _ = loadSoup() orelse return;
    _ = ext.cookie_manager_delete_cookie orelse return;
    const get_all = ext.cookie_manager_get_all_cookies orelse return;
    const cm = cookieManager(v) orelse return;
    const obj = cookieArg(arg) orelse return;
    const name = objStr(obj, "name") orelse {
        std.debug.print("ND_WARN WebView deleteCookie: missing name\n", .{});
        return;
    };
    const ctx = alloc.create(DeleteCookieCtx) catch return;
    ctx.* = .{
        .name = alloc.dupe(u8, name) catch {
            alloc.destroy(ctx);
            return;
        },
        .domain = if (objStr(obj, "domain")) |d| (alloc.dupe(u8, d) catch null) else null,
        .path = if (objStr(obj, "path")) |p| (alloc.dupe(u8, p) catch null) else null,
    };
    get_all(cm, null, &cbDeleteMatchingCookies, ctx);
}

fn cbDeleteMatchingCookies(source: ?*anyopaque, res: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    const ctx: *DeleteCookieCtx = @ptrCast(@alignCast(data orelse return));
    defer {
        alloc.free(ctx.name);
        if (ctx.domain) |d| alloc.free(d);
        if (ctx.path) |p| alloc.free(p);
        alloc.destroy(ctx);
    }
    const cm = source orelse return;
    const finish = ext.cookie_manager_get_all_cookies_finish orelse return;
    const del = ext.cookie_manager_delete_cookie orelse return;
    const s = loadSoup() orelse return;

    var err: ?*glib.Error = null;
    const list = finish(cm, res, &err) orelse {
        if (err) |e| {
            defer e.free();
            const msg: []const u8 = if (e.f_message) |m| std.mem.span(m) else "unknown error";
            std.debug.print("ND_WARN WebView deleteCookie: could not read the cookie jar: {s}\n", .{msg});
        }
        return;
    };
    var node: ?*glib.List = list;
    while (node) |n| : (node = n.f_next) {
        const cookie = n.f_data orelse continue;
        const name = if (s.cookie_get_name(cookie)) |x| std.mem.span(x) else continue;
        if (!std.mem.eql(u8, name, ctx.name)) continue;
        if (ctx.domain) |want| {
            const have: []const u8 = if (s.cookie_get_domain(cookie)) |x| std.mem.span(x) else "";
            // Stored host cookies keep a leading dot for domain cookies; the
            // app names the host, so compare both spellings.
            const have_bare = if (have.len > 0 and have[0] == '.') have[1..] else have;
            if (!std.mem.eql(u8, have_bare, want)) continue;
        }
        if (ctx.path) |want| {
            const have: []const u8 = if (s.cookie_get_path(cookie)) |x| std.mem.span(x) else "";
            if (!std.mem.eql(u8, have, want)) continue;
        }
        del(cm, cookie, null, &cbCookieDeleted, null);
    }
    glib.List.freeFull(list, @ptrCast(s.cookie_free));
}

/// add/delete report through GAsyncResult; the finish call has to run or GIO
/// leaks the result, and a failure is worth one warning line.
fn cbCookieAdded(source: ?*anyopaque, res: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
    reportCookieWrite(ext.cookie_manager_add_cookie_finish, source, res, "setCookie");
}

fn cbCookieDeleted(source: ?*anyopaque, res: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
    reportCookieWrite(ext.cookie_manager_delete_cookie_finish, source, res, "deleteCookie");
}

fn reportCookieWrite(finish_fn: ?FnCookieMutateFinish, source: ?*anyopaque, res: ?*anyopaque, what: []const u8) void {
    const src = source orelse return;
    const finish = finish_fn orelse return;
    var err: ?*glib.Error = null;
    _ = finish(src, res, &err);
    if (err) |e| {
        defer e.free();
        const msg: []const u8 = if (e.f_message) |m| std.mem.span(m) else "unknown error";
        std.debug.print("ND_WARN WebView {s} failed: {s}\n", .{ what, msg });
    }
}

// ============================================================================
// Find in page
// ============================================================================

fn findOptions(obj: ?std.json.ObjectMap) u32 {
    var options: u32 = 0;
    const case_sensitive = if (obj) |o| (objBool(o, "caseSensitive") orelse false) else false;
    if (!case_sensitive) options |= FIND_CASE_INSENSITIVE;
    const wrap = if (obj) |o| (objBool(o, "wrap") orelse true) else true;
    if (wrap) options |= FIND_WRAP_AROUND;
    return options;
}

fn cmdFindStart(v: *anyopaque, arg: ?std.json.Value) void {
    const get_fc = ext.get_find_controller orelse return;
    const search = ext.find_search orelse return;
    const obj = argObject(arg) orelse return;
    const text = objStr(obj, "text") orelse {
        std.debug.print("ND_WARN WebView findStart: missing text\n", .{});
        return;
    };
    const fc = get_fc(v) orelse return;
    const text_z = alloc.dupeZ(u8, text) catch return;
    defer alloc.free(text_z);
    const options = findOptions(obj);
    search(fc, text_z.ptr, options, FIND_MAX_MATCHES);
    // `search` reports found/not-found; the total only arrives through a
    // separate count pass, so both run and the app gets two findResult events.
    if (ext.find_count_matches) |count| count(fc, text_z.ptr, options, FIND_MAX_MATCHES);
}

fn cmdFindStep(v: *anyopaque, step: ?FnVoidOnPtr) void {
    const get_fc = ext.get_find_controller orelse return;
    const f = step orelse return;
    const fc = get_fc(v) orelse return;
    f(fc);
}

// ============================================================================
// Session state
// ============================================================================

fn cmdSaveSession(widget: *gtk.Widget, v: *anyopaque, arg: ?std.json.Value) void {
    const get_state = ext.get_session_state orelse return;
    const serialize = ext.session_state_serialize orelse return;
    const node_id = widgetNodeId(widget) orelse return;
    const id = if (argObject(arg)) |o| (objStr(o, "id") orelse "") else "";
    const session_state = get_state(v) orelse return;
    defer if (ext.session_state_unref) |unref| unref(session_state);
    const bytes = serialize(session_state) orelse return;
    defer bytes.unref();
    var size: usize = 0;
    const data = bytes.getData(&size) orelse return;

    const encoder = std.base64.standard.Encoder;
    const out = alloc.alloc(u8, encoder.calcSize(size)) catch return;
    defer alloc.free(out);
    const encoded = encoder.encode(out, data[0..size]);

    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    objPutStr(&payload, "id", id);
    objPutStr(&payload, "state", encoded);
    emitObject(node_id, "sessionSaved", payload);
}

fn cmdRestoreSession(v: *anyopaque, arg: ?std.json.Value) void {
    const state_new = ext.session_state_new orelse return;
    const restore = ext.restore_session_state orelse return;
    const obj = argObject(arg) orelse return;
    const b64 = objStr(obj, "state") orelse {
        std.debug.print("ND_WARN WebView restoreSession: missing state\n", .{});
        return;
    };
    const decoder = std.base64.standard.Decoder;
    const size = decoder.calcSizeForSlice(b64) catch {
        std.debug.print("ND_WARN WebView restoreSession: malformed base64 state\n", .{});
        return;
    };
    const buf = alloc.alloc(u8, size) catch return;
    defer alloc.free(buf);
    decoder.decode(buf, b64) catch {
        std.debug.print("ND_WARN WebView restoreSession: malformed base64 state\n", .{});
        return;
    };
    const bytes = glib.Bytes.new(buf.ptr, size);
    defer bytes.unref();
    const session_state = state_new(bytes) orelse return;
    defer if (ext.session_state_unref) |unref| unref(session_state);
    restore(v, session_state);
    // Restoring only rebuilds the back/forward list — navigating to its
    // current item is what actually puts the page back on screen.
    const get_bf = ext.get_back_forward_list orelse return;
    const current = ext.bf_list_get_current_item orelse return;
    const go_to = ext.go_to_bf_list_item orelse return;
    const list = get_bf(v) orelse return;
    const item = current(list) orelse return;
    go_to(v, item);
}

// ============================================================================
// Audio
// ============================================================================

fn emitAudioState(node_id: u32, v: *anyopaque) void {
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    objPutBool(&payload, "playing", if (ext.is_playing_audio) |f| f(v) != 0 else false);
    objPutBool(&payload, "muted", if (ext.get_is_muted) |f| f(v) != 0 else false);
    emitObject(node_id, "audioStateChanged", payload);
}

fn cmdSetMuted(widget: *gtk.Widget, v: *anyopaque, arg: ?std.json.Value) void {
    const set_muted = ext.set_is_muted orelse return;
    const muted = switch (arg orelse return) {
        .bool => |b| b,
        .object => |o| objBool(o, "muted") orelse return,
        else => return,
    };
    set_muted(v, if (muted) 1 else 0);
    if (widgetNodeId(widget)) |node_id| emitAudioState(node_id, v);
}

// ============================================================================
// Context menu
// ============================================================================

/// The app's item tree for this view, replacing whatever it held before. The
/// tree is stored, not rendered: which of these items a right-click earns is
/// decided per invocation against that click's hit test.
fn cmdSetContextMenuItems(widget: *gtk.Widget, arg: ?std.json.Value) void {
    const state = stateOf(widget) orelse return;
    const items = ctxmenu.parse(alloc, arg orelse .null) catch {
        std.debug.print("ND_WARN WebView setContextMenuItems: out of memory, items unchanged\n", .{});
        return;
    };
    ctxmenu.freeItems(alloc, state.menu_items);
    state.menu_items = items;
    tr("setContextMenuItems node={?d} items={d}", .{ widgetNodeId(widget), items.len });
}

/// Everything an activated item needs to describe the click it came from.
/// Allocated per item per menu invocation; freed by the closure's destroy
/// notify, which runs when WebKit drops the menu and with it the GAction.
const MenuClick = struct {
    node_id: u32,
    /// The item's id, or for a radio run the id that was checked when the
    /// menu opened. The clicked id then arrives as the action's target.
    id: []const u8,
    kind: ctxmenu.Kind,
    checked: bool,
    page_url: []const u8,
    link: []const u8,
    image: []const u8,
    selection: []const u8,
    editable: bool,
};

fn dupeForClick(s: []const u8) []const u8 {
    if (s.len == 0) return "";
    return alloc.dupe(u8, s) catch "";
}

fn freeForClick(s: []const u8) void {
    if (s.len > 0) alloc.free(s);
}

fn freeMenuClick(data: ?*anyopaque, _: *anyopaque) callconv(.c) void {
    const click: *MenuClick = @ptrCast(@alignCast(data orelse return));
    freeForClick(click.id);
    freeForClick(click.page_url);
    freeForClick(click.link);
    freeForClick(click.image);
    freeForClick(click.selection);
    alloc.destroy(click);
}

fn emitMenuClick(click: *const MenuClick, id: []const u8, checked: ?bool, was_checked: ?bool) void {
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    objPutStr(&payload, "id", id);
    objPutStr(&payload, "pageUrl", click.page_url);
    if (click.link.len > 0) objPutStr(&payload, "linkUrl", click.link);
    if (click.image.len > 0) objPutStr(&payload, "imageUrl", click.image);
    if (click.selection.len > 0) objPutStr(&payload, "selectionText", click.selection);
    objPutBool(&payload, "editable", click.editable);
    if (checked) |v| objPutBool(&payload, "checked", v);
    if (was_checked) |v| objPutBool(&payload, "wasChecked", v);
    emitObject(click.node_id, "contextMenuItemClicked", payload);
}

/// GSimpleAction::activate for one of the app's items. The framework reports
/// the new state a checkbox or radio click implies and does NOT mutate its own
/// copy of the tree: the app owns the model and answers with the next
/// `setContextMenuItems`.
fn cbMenuAction(_: *gio.SimpleAction, param: ?*glib.Variant, data: ?*anyopaque) callconv(.c) void {
    const click: *MenuClick = @ptrCast(@alignCast(data orelse return));
    switch (click.kind) {
        .checkbox => emitMenuClick(click, click.id, !click.checked, click.checked),
        .radio => {
            const target = param orelse return emitMenuClick(click, click.id, true, click.checked);
            const clicked = std.mem.span(glib.Variant.getString(target, null));
            emitMenuClick(click, clicked, true, std.mem.eql(u8, clicked, click.id));
        },
        else => emitMenuClick(click, click.id, null, null),
    }
}

const MenuBuild = struct {
    node_id: u32,
    hit: ctxmenu.Hit,
    page_url: []const u8,
    /// GSimpleAction names have a grammar the app's ids do not, so items get
    /// generated names and carry their real id in the click context.
    counter: u32 = 0,
};

fn newClick(b: *const MenuBuild, item: ctxmenu.Item, id: []const u8) ?*MenuClick {
    const click = alloc.create(MenuClick) catch return null;
    click.* = .{
        .node_id = b.node_id,
        .id = dupeForClick(id),
        .kind = item.kind,
        .checked = item.checked,
        .page_url = dupeForClick(b.page_url),
        .link = dupeForClick(b.hit.link),
        .image = dupeForClick(b.hit.image),
        .selection = dupeForClick(b.hit.selection),
        .editable = b.hit.editable,
    };
    return click;
}

fn newAction(b: *MenuBuild, item: ctxmenu.Item, id: []const u8, param_type: ?*glib.VariantType, state: ?*glib.Variant) ?*gio.SimpleAction {
    var name_buf: [32]u8 = undefined;
    const name = std.fmt.bufPrintZ(&name_buf, "nd-ctx-{d}", .{b.counter}) catch return null;
    b.counter += 1;
    const action = if (state) |s|
        gio.SimpleAction.newStateful(name.ptr, param_type, s)
    else
        gio.SimpleAction.new(name.ptr, param_type);
    if (!item.enabled) gio.SimpleAction.setEnabled(action, 0);
    const click = newClick(b, item, id) orelse {
        gobject.Object.unref(@ptrCast(@alignCast(action)));
        return null;
    };
    _ = gobject.signalConnectData(@ptrCast(@alignCast(action)), "activate", @ptrCast(&cbMenuAction), click, freeMenuClick, .{});
    return action;
}

fn appendSeparator(menu: *anyopaque) void {
    const append = ext.context_menu_append orelse return;
    const mk = ext.context_menu_item_new_separator orelse return;
    const item = mk() orelse return;
    append(menu, item);
}

/// A run of contiguous radio siblings is ONE stateful action: that is what
/// makes GTK draw radios rather than checkmarks, and it is also Chrome's
/// grouping rule for `type: "radio"`.
fn appendRadioRun(b: *MenuBuild, menu: *anyopaque, items: []const ctxmenu.Item, start: usize) usize {
    var end = start;
    while (end < items.len and items[end].kind == .radio) end += 1;
    const append = ext.context_menu_append orelse return end;
    const from_action = ext.context_menu_item_new_from_gaction orelse return end;

    var checked_id: []const u8 = "";
    for (items[start..end]) |item| {
        if (item.checked) {
            checked_id = item.id;
            break;
        }
    }
    const type_s = glib.VariantType.new("s");
    defer glib.VariantType.free(type_s);
    const checked_z = alloc.dupeZ(u8, checked_id) catch return end;
    defer alloc.free(checked_z);
    const action = newAction(b, items[start], checked_id, type_s, glib.Variant.newString(checked_z.ptr)) orelse return end;
    defer gobject.Object.unref(@ptrCast(@alignCast(action)));

    for (items[start..end]) |item| {
        if (!ctxmenu.matches(item, b.hit)) continue;
        const target_z = alloc.dupeZ(u8, item.id) catch continue;
        defer alloc.free(target_z);
        const mi = from_action(@ptrCast(action), item.label.ptr, glib.Variant.newString(target_z.ptr)) orelse continue;
        append(menu, mi);
    }
    return end;
}

fn appendOne(b: *MenuBuild, menu: *anyopaque, item: ctxmenu.Item) void {
    const append = ext.context_menu_append orelse return;
    if (item.children.len > 0) {
        const new_menu = ext.context_menu_new orelse return;
        const with_submenu = ext.context_menu_item_new_with_submenu orelse return;
        const sub = new_menu() orelse return;
        // Both the submenu and the action below are `transfer none` arguments
        // (WebKit-6.0.gir): WebKit takes its own reference, so the one this
        // call owns has to be dropped here.
        defer gobject.Object.unref(@ptrCast(@alignCast(sub)));
        _ = appendItems(b, sub, item.children);
        const mi = with_submenu(item.label.ptr, sub) orelse return;
        append(menu, mi);
        return;
    }
    const from_action = ext.context_menu_item_new_from_gaction orelse return;
    const state: ?*glib.Variant = if (item.kind == .checkbox)
        glib.Variant.newBoolean(if (item.checked) 1 else 0)
    else
        null;
    const action = newAction(b, item, item.id, null, state) orelse return;
    defer gobject.Object.unref(@ptrCast(@alignCast(action)));
    const mi = from_action(@ptrCast(action), item.label.ptr, null) orelse return;
    append(menu, mi);
}

/// Appends the items this hit earned, and returns how many landed. Separators
/// are held back until something follows them, so a filtered-out neighbour can
/// never leave a menu opening or ending on a rule.
fn appendItems(b: *MenuBuild, menu: *anyopaque, items: []const ctxmenu.Item) usize {
    var appended: usize = 0;
    var pending_separator = false;
    var i: usize = 0;
    while (i < items.len) {
        const item = items[i];
        if (item.kind == .separator) {
            if (appended > 0) pending_separator = true;
            i += 1;
            continue;
        }
        if (!ctxmenu.survives(item, b.hit)) {
            i += 1;
            continue;
        }
        if (pending_separator) {
            appendSeparator(menu);
            pending_separator = false;
        }
        if (item.kind == .radio) {
            i = appendRadioRun(b, menu, items, i);
        } else {
            appendOne(b, menu, item);
            i += 1;
        }
        appended += 1;
    }
    return appended;
}

/// Merges the app's items into WebKit's own menu: one separator after the
/// stock items, then whatever this click earned.
fn mergeCustomItems(state: *ViewState, node_id: u32, menu: *anyopaque, hit: ctxmenu.Hit, page_url: []const u8) void {
    // A separator with nothing after it is worse than no items at all, so the
    // hit test decides before anything is built.
    var any = false;
    for (state.menu_items) |item| {
        if (item.kind == .separator) continue;
        if (ctxmenu.survives(item, hit)) {
            any = true;
            break;
        }
    }
    if (!any) return;
    var b = MenuBuild{ .node_id = node_id, .hit = hit, .page_url = page_url };
    appendSeparator(menu);
    _ = appendItems(&b, menu, state.menu_items);
}

// ============================================================================
// Command dispatch
// ============================================================================

/// widgetCommand dispatch (generated widgets.zig WebView arm).
pub fn command(widget: *gtk.Widget, cmd: []const u8, arg: ?std.json.Value) void {
    const a = api orelse return;
    if (!isReal(widget)) return;
    const v: *anyopaque = @ptrCast(widget);
    if (std.mem.eql(u8, cmd, "goBack")) {
        if (a.web_view_can_go_back(v) != 0) a.web_view_go_back(v);
    } else if (std.mem.eql(u8, cmd, "goForward")) {
        if (a.web_view_can_go_forward(v) != 0) a.web_view_go_forward(v);
    } else if (std.mem.eql(u8, cmd, "reload")) {
        a.web_view_reload(v);
    } else if (std.mem.eql(u8, cmd, "stop")) {
        a.web_view_stop_loading(v);
    } else if (std.mem.eql(u8, cmd, "setZoom")) {
        cmdSetZoom(v, arg);
    } else if (std.mem.eql(u8, cmd, "setUserAgent")) {
        cmdSetUserAgent(v, arg);
    } else if (std.mem.eql(u8, cmd, "openDevTools")) {
        cmdOpenDevTools(v);
    } else if (std.mem.eql(u8, cmd, "executeJavaScript")) {
        cmdExecuteJavaScript(widget, v, arg);
    } else if (std.mem.eql(u8, cmd, "addUserScript")) {
        cmdAddUserScript(widget, v, arg);
    } else if (std.mem.eql(u8, cmd, "removeUserScript")) {
        cmdRemoveUserScript(widget, v, arg);
    } else if (std.mem.eql(u8, cmd, "clearUserScripts")) {
        cmdClearUserScripts(widget, v, arg);
    } else if (std.mem.eql(u8, cmd, "registerScriptMessage")) {
        cmdRegisterScriptMessage(widget, v, arg);
    } else if (std.mem.eql(u8, cmd, "unregisterScriptMessage")) {
        cmdUnregisterScriptMessage(widget, v, arg);
    } else if (std.mem.eql(u8, cmd, "respondScheme")) {
        cmdRespondScheme(arg);
    } else if (std.mem.eql(u8, cmd, "getCookies")) {
        cmdGetCookies(widget, v, arg);
    } else if (std.mem.eql(u8, cmd, "setCookie")) {
        cmdSetCookie(v, arg);
    } else if (std.mem.eql(u8, cmd, "deleteCookie")) {
        cmdDeleteCookie(v, arg);
    } else if (std.mem.eql(u8, cmd, "findStart")) {
        cmdFindStart(v, arg);
    } else if (std.mem.eql(u8, cmd, "findNext")) {
        cmdFindStep(v, ext.find_search_next);
    } else if (std.mem.eql(u8, cmd, "findPrevious")) {
        cmdFindStep(v, ext.find_search_previous);
    } else if (std.mem.eql(u8, cmd, "findStop")) {
        cmdFindStep(v, ext.find_search_finish);
    } else if (std.mem.eql(u8, cmd, "saveSession")) {
        cmdSaveSession(widget, v, arg);
    } else if (std.mem.eql(u8, cmd, "restoreSession")) {
        cmdRestoreSession(v, arg);
    } else if (std.mem.eql(u8, cmd, "setMuted")) {
        cmdSetMuted(widget, v, arg);
    } else if (std.mem.eql(u8, cmd, "setContextMenuItems")) {
        cmdSetContextMenuItems(widget, arg);
    } else {
        std.debug.print("ND_WARN unknown WebView command {s}\n", .{cmd});
    }
}

// ============================================================================
// Signal wiring
// ============================================================================

/// Generated connectEvents WebView arm: wires the WebKit signals that feed
/// the schema events. `emit_fn` is the generated module's emit sink (same
/// EmitFn shape, installed before any widget exists).
pub fn connectEvents(widget: *gtk.Widget, node_id: u32, emit_fn: EmitFn) void {
    if (api == null or !isReal(widget)) return;
    emit = emit_fn;
    any_view_created = true;
    const obj = widget.as(gobject.Object);
    const data: ?*anyopaque = @ptrFromInt(@as(usize, node_id));
    gobject.Object.setData(obj, NODE_ID_KEY, data); // retrieved by widgetCommand, which gets no node_id of its own
    if (stateOf(widget)) |state| state.node_id = node_id;
    _ = gobject.signalConnectData(obj, "load-changed", @ptrCast(&cbLoadChanged), data, null, .{});
    _ = gobject.signalConnectData(obj, "notify::uri", @ptrCast(&cbNotifyUri), data, null, .{});
    _ = gobject.signalConnectData(obj, "notify::title", @ptrCast(&cbNotifyTitle), data, null, .{});
    _ = gobject.signalConnectData(obj, "load-failed", @ptrCast(&cbLoadFailed), data, null, .{});
    if (ext.get_estimated_load_progress != null) {
        _ = gobject.signalConnectData(obj, "notify::estimated-load-progress", @ptrCast(&cbNotifyProgress), data, null, .{});
    }
    if (ext.navigation_action_get_request != null and ext.uri_request_get_uri != null) {
        _ = gobject.signalConnectData(obj, "create", @ptrCast(&cbCreate), data, null, .{});
    }
    if (ext.get_network_session != null and ext.download_get_request != null and ext.uri_request_get_uri != null and ext.download_cancel != null) {
        if (ext.get_network_session.?(@ptrCast(widget))) |session| connectDownloads(session);
    }
    if (ext.get_favicon != null) {
        // The favicon database is off by default, and `notify::favicon` never
        // fires without it — turning it on is part of wiring the event.
        if (ext.get_network_session) |get_session| {
            if (ext.network_session_get_website_data_manager) |get_manager| {
                if (ext.website_data_manager_set_favicons_enabled) |enable| {
                    if (get_session(@ptrCast(widget))) |session| {
                        if (get_manager(session)) |manager| enable(manager, 1);
                    }
                }
            }
        }
        _ = gobject.signalConnectData(obj, "notify::favicon", @ptrCast(&cbNotifyFavicon), data, null, .{});
    }
    _ = gobject.signalConnectData(obj, "insecure-content-detected", @ptrCast(&cbInsecureContent), data, null, .{});
    _ = gobject.signalConnectData(obj, "load-failed-with-tls-errors", @ptrCast(&cbTlsErrors), data, null, .{});
    if (ext.hit_test_get_link_uri != null) {
        _ = gobject.signalConnectData(obj, "mouse-target-changed", @ptrCast(&cbMouseTarget), data, null, .{});
    }
    _ = gobject.signalConnectData(obj, "context-menu", @ptrCast(&cbContextMenu), data, null, .{});
    if (ext.is_playing_audio != null) {
        _ = gobject.signalConnectData(obj, "notify::is-playing-audio", @ptrCast(&cbNotifyAudio), data, null, .{});
    }
    if (ext.get_find_controller) |get_fc| {
        if (get_fc(@ptrCast(widget))) |fc| {
            _ = gobject.signalConnectData(@ptrCast(@alignCast(fc)), "found-text", @ptrCast(&cbFoundText), data, null, .{});
            _ = gobject.signalConnectData(@ptrCast(@alignCast(fc)), "failed-to-find-text", @ptrCast(&cbFailedToFind), data, null, .{});
            _ = gobject.signalConnectData(@ptrCast(@alignCast(fc)), "counted-matches", @ptrCast(&cbCountedMatches), data, null, .{});
        }
    }
    if (cookieManager(@ptrCast(widget))) |cm| {
        _ = gobject.signalConnectData(@ptrCast(@alignCast(cm)), "changed", @ptrCast(&cbCookiesChanged), data, null, .{});
    }
}

fn cbLoadChanged(obj: *gobject.Object, load_event: c_int, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const a = api orelse return;
    const v: *anyopaque = @ptrCast(obj);
    tr("load node={d} event={d} uri={s}", .{ node_id, load_event, if (a.web_view_get_uri(v)) |u| std.mem.span(u) else "" });
    if (emit) |f| {
        f(node_id, "loadingChanged", .{ .checked = load_event != WEBKIT_LOAD_FINISHED });
        // History availability changes exactly on load transitions.
        f(node_id, "backAvailable", .{ .checked = a.web_view_can_go_back(v) != 0 });
        f(node_id, "forwardAvailable", .{ .checked = a.web_view_can_go_forward(v) != 0 });
    }
    // TLS state is only meaningful once the navigation has committed.
    if (load_event == WEBKIT_LOAD_COMMITTED) emitSecurity(node_id, v);
}

/// `securityChanged` on commit: `secure` means the page came over TLS with no
/// certificate errors. `get_tls_info` returning FALSE is the plain-http case.
fn emitSecurity(node_id: u32, v: *anyopaque) void {
    const get_tls = ext.get_tls_info orelse return;
    var flags: c_uint = 0;
    const has_tls = get_tls(v, null, &flags) != 0;
    const secure = has_tls and flags == 0;
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    objPutBool(&payload, "secure", secure);
    objPutBool(&payload, "insecureContent", false);
    if (has_tls and flags != 0) {
        var buf: [64]u8 = undefined;
        const msg = std.fmt.bufPrint(&buf, "TLS certificate errors (flags 0x{x})", .{flags}) catch "TLS certificate errors";
        objPutStr(&payload, "error", msg);
        emitObject(node_id, "securityChanged", payload);
        return;
    }
    emitObject(node_id, "securityChanged", payload);
}

fn cbInsecureContent(_: *gobject.Object, _: c_int, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    objPutBool(&payload, "secure", false);
    objPutBool(&payload, "insecureContent", true);
    emitObject(node_id, "securityChanged", payload);
}

/// Returning FALSE keeps WebKit's refusal to load a page with TLS errors —
/// the framework never silently downgrades a certificate failure. The app
/// gets the event and decides what to show.
fn cbTlsErrors(_: *gobject.Object, failing_uri: ?[*:0]const u8, _: ?*anyopaque, errors: c_uint, data: ?*anyopaque) callconv(.c) c_int {
    const node_id: u32 = @intCast(@intFromPtr(data));
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    objPutBool(&payload, "secure", false);
    objPutBool(&payload, "insecureContent", false);
    objPutStr(&payload, "url", if (failing_uri) |u| std.mem.span(u) else "");
    var buf: [64]u8 = undefined;
    const msg = std.fmt.bufPrint(&buf, "TLS certificate errors (flags 0x{x})", .{errors}) catch "TLS certificate errors";
    objPutStr(&payload, "error", msg);
    emitObject(node_id, "securityChanged", payload);
    return 0;
}

// notify:: handlers get (object, pspec, user_data).
fn cbNotifyUri(obj: *gobject.Object, _: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const a = api orelse return;
    const uri = a.web_view_get_uri(@ptrCast(obj)) orelse return;
    if (emit) |f| f(node_id, "navigate", .{ .text = std.mem.span(uri) });
}

fn cbNotifyTitle(obj: *gobject.Object, _: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const a = api orelse return;
    const title = a.web_view_get_title(@ptrCast(obj)) orelse return;
    if (emit) |f| f(node_id, "titleChanged", .{ .text = std.mem.span(title) });
}

/// Progress is stored bitcast into the widget's GObject data (same idiom as
/// MARKER_KEY) so the handler can skip re-emitting an unchanged value; -1.0
/// (never a real progress value) marks "nothing emitted yet". 64-bit targets
/// only — f64 and usize/pointer are both 8 bytes, which is all this project
/// builds for (macOS/Linux, no 32-bit backend).
fn cbNotifyProgress(obj: *gobject.Object, _: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const get_progress = ext.get_estimated_load_progress orelse return;
    const progress = get_progress(@ptrCast(obj));
    const prev: f64 = if (gobject.Object.getData(obj, PROGRESS_KEY)) |p| @bitCast(@intFromPtr(p)) else -1.0;
    if (progress == prev) return;
    gobject.Object.setData(obj, PROGRESS_KEY, @ptrFromInt(@as(usize, @bitCast(progress))));
    if (emit) |f| f(node_id, "loadProgress", .{ .value = progress });
}

/// `notify::favicon`: WebKit hands over a GdkTexture, which GDK can encode to
/// PNG without any extra dependency. Oversized icons are skipped rather than
/// pushed through NDP — apps that want them can fetch `iconUrl` themselves.
fn cbNotifyFavicon(obj: *gobject.Object, _: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const get_favicon = ext.get_favicon orelse return;
    const raw = get_favicon(@ptrCast(obj)) orelse return;
    const texture: *gdk.Texture = @ptrCast(@alignCast(raw));
    const bytes = gdk.Texture.saveToPngBytes(texture);
    defer bytes.unref();
    var size: usize = 0;
    const png = bytes.getData(&size) orelse return;
    if (size == 0 or size > FAVICON_MAX_PNG_BYTES) {
        if (size > FAVICON_MAX_PNG_BYTES) {
            std.debug.print("ND_WARN WebView faviconChanged: icon of {d} bytes exceeds the {d}-byte cap; skipped\n", .{ size, FAVICON_MAX_PNG_BYTES });
        }
        return;
    }
    const prefix = "data:image/png;base64,";
    const encoder = std.base64.standard.Encoder;
    const out = alloc.alloc(u8, prefix.len + encoder.calcSize(size)) catch return;
    defer alloc.free(out);
    @memcpy(out[0..prefix.len], prefix);
    _ = encoder.encode(out[prefix.len..], png[0..size]);

    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    objPutStr(&payload, "dataUrl", out);
    emitObject(node_id, "faviconChanged", payload);
}

fn cbMouseTarget(_: *gobject.Object, hit: ?*anyopaque, _: c_uint, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const get_link = ext.hit_test_get_link_uri orelse return;
    const h = hit orelse return;
    const url: []const u8 = if (get_link(h)) |u| std.mem.span(u) else "";
    if (emit) |f| f(node_id, "linkHover", .{ .text = url });
}

/// `context-menu`: returning TRUE suppresses WebKit's own menu, which is what
/// `contextMenuMode="suppress"` buys: the app then shows a native menu off
/// this event's hit-test data. In `native` mode the engine menu stays (stock
/// items, spell-check, Inspect Element when developer extras are on) and the
/// app's `setContextMenuItems` are appended to it in place.
///
/// The informational `contextMenu` event fires in BOTH modes: the hit test is
/// already read here, so an app that wants to know costs nothing.
fn cbContextMenu(obj: *gobject.Object, menu: ?*anyopaque, hit: ?*anyopaque, data: ?*anyopaque) callconv(.c) c_int {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const widget: *gtk.Widget = @ptrCast(obj);
    const state = stateOf(widget);
    const mode = if (state) |s| s.mode else .native;

    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    var x: c_int = 0;
    var y: c_int = 0;
    if (ext.context_menu_get_position) |get_pos| {
        if (menu) |m| _ = get_pos(m, &x, &y);
    }
    objPutInt(&payload, "x", x);
    objPutInt(&payload, "y", y);
    var found = ctxmenu.Hit{};
    if (hit) |h| {
        if (ext.hit_test_get_context) |get_ctx| {
            const context = get_ctx(h);
            found.editable = (context & HIT_EDITABLE) != 0;
            found.has_selection = (context & HIT_SELECTION) != 0;
        }
        if (ext.hit_test_get_link_uri) |get_link| {
            if (get_link(h)) |u| found.link = std.mem.span(u);
        }
        if (ext.hit_test_get_image_uri) |get_image| {
            if (get_image(h)) |u| found.image = std.mem.span(u);
        }
    }
    if (found.link.len > 0) objPutStr(&payload, "link", found.link);
    if (found.image.len > 0) objPutStr(&payload, "image", found.image);
    objPutBool(&payload, "editable", found.editable);
    // WebKitGTK's hit test reports THAT there is a selection but never its
    // text, so `selection` is AppKit-only; `hasSelection` is the portable flag.
    objPutBool(&payload, "hasSelection", found.has_selection);
    emitObject(node_id, "contextMenu", payload);

    if (mode == .suppress) return 1;
    if (state) |s| {
        if (menu) |m| {
            const page_url: []const u8 = blk: {
                const a = api orelse break :blk "";
                break :blk if (a.web_view_get_uri(@ptrCast(widget))) |u| std.mem.span(u) else "";
            };
            mergeCustomItems(s, node_id, m, found, page_url);
        }
    }
    return 0;
}

fn cbNotifyAudio(obj: *gobject.Object, _: ?*anyopaque, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    emitAudioState(node_id, @ptrCast(obj));
}

/// `done` separates the outcome of a find operation (search/next/previous)
/// from the asynchronous match-count update WebKitGTK delivers separately.
/// `matchCount` is GTK-only: WKFindResult on AppKit reports match/no-match
/// without a total, so apps must treat the field as optional.
fn emitFindResult(node_id: u32, match_count: u32, match_found: bool, done: bool) void {
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    objPutBool(&payload, "matchFound", match_found);
    objPutInt(&payload, "matchCount", match_count);
    objPutBool(&payload, "done", done);
    emitObject(node_id, "findResult", payload);
}

fn cbFoundText(_: *gobject.Object, match_count: c_uint, data: ?*anyopaque) callconv(.c) void {
    emitFindResult(@intCast(@intFromPtr(data)), @intCast(match_count), true, true);
}

fn cbFailedToFind(_: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    emitFindResult(@intCast(@intFromPtr(data)), 0, false, true);
}

fn cbCountedMatches(_: *gobject.Object, match_count: c_uint, data: ?*anyopaque) callconv(.c) void {
    emitFindResult(@intCast(@intFromPtr(data)), @intCast(match_count), match_count > 0, false);
}

/// The cookie manager's `changed` signal carries no payload — it's a "re-read
/// the jar" ping, so the event is a bare object.
fn cbCookiesChanged(_: *gobject.Object, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    emitObject(node_id, "cookiesChanged", payload);
}

/// `load-failed`: returning FALSE keeps WebKit's built-in error page (apps
/// get the event for their own handling — e.g. a toast — without losing the
/// safety net of a page that isn't just blank).
fn cbLoadFailed(_: *gobject.Object, _: c_int, failing_uri: ?[*:0]const u8, err: ?*glib.Error, data: ?*anyopaque) callconv(.c) c_int {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const e = err orelse return 0;
    if (ext.network_error_quark) |quark_fn| {
        if (e.f_domain == quark_fn() and e.f_code == WEBKIT_NETWORK_ERROR_CANCELLED) return 0;
    }
    if (ext.policy_error_quark) |quark_fn| {
        if (e.f_domain == quark_fn() and e.f_code == WEBKIT_POLICY_ERROR_FRAME_LOAD_INTERRUPTED_BY_POLICY_CHANGE) return 0;
    }
    const f = emit orelse return 0;
    const url: []const u8 = if (failing_uri) |u| std.mem.span(u) else "";
    const msg: []const u8 = if (e.f_message) |m| std.mem.span(m) else "";
    var payload: std.json.ObjectMap = .empty;
    defer payload.deinit(alloc);
    objPutStr(&payload, "url", url);
    objPutStr(&payload, "error", msg);
    f(node_id, "loadFailed", .{ .data = .{ .object = payload } });
    return 0;
}

/// `create`: fired for window.open/target=_blank. Returning NULL means no
/// native window is created — the app is expected to open a native tab
/// itself off the emitted URL.
fn cbCreate(_: *gobject.Object, navigation_action: ?*anyopaque, data: ?*anyopaque) callconv(.c) ?*gtk.Widget {
    const node_id: u32 = @intCast(@intFromPtr(data));
    const get_request = ext.navigation_action_get_request orelse return null;
    const get_uri = ext.uri_request_get_uri orelse return null;
    const nav = navigation_action orelse return null;
    const request = get_request(nav) orelse return null;
    const uri = get_uri(request) orelse return null;
    if (emit) |f| f(node_id, "newWindow", .{ .text = std.mem.span(uri) });
    return null;
}

/// Sessions already carrying a `download-started` handler, keyed by pointer.
/// The signal lives on the SESSION, and every view without a `profile` shares
/// the default one, so connecting per view meant one download ran the handler
/// once per live webview: N `downloadRequested` events for one download, and N
/// `webkit_download_cancel` calls on the same proxy — the second completion
/// lands in `DownloadProxy::cancel` on freed memory and takes the host down.
var download_sessions: std.AutoHashMapUnmanaged(usize, void) = .empty;

fn connectDownloads(session: *anyopaque) void {
    const key = @intFromPtr(session);
    if (download_sessions.contains(key)) return;
    download_sessions.put(alloc, key, {}) catch return;
    _ = gobject.signalConnectData(@ptrCast(@alignCast(session)), "download-started", @ptrCast(&cbDownloadStarted), null, null, .{});
}

/// `download-started` on the WebKitNetworkSession (WebKitGTK 6 moved
/// downloads off WebKitWebContext). Connected once per session (see
/// `download_sessions`), so the download names its own view rather than the
/// closure carrying a node id.
///
/// The report is deferred to `decide-destination` rather than made here: the
/// response headers have not landed yet at download-started, so
/// `suggestedFilename` would always be null. `decide-destination` fires with
/// the engine's own suggested name and BEFORE any byte reaches disk, so
/// cancelling from there still costs nothing.
fn cbDownloadStarted(_: *gobject.Object, download: ?*anyopaque, _: ?*anyopaque) callconv(.c) void {
    const dl = download orelse return;
    _ = gobject.signalConnectData(@ptrCast(@alignCast(dl)), "decide-destination", @ptrCast(&cbDecideDestination), null, null, .{});
}

/// `WebKitDownload::decide-destination`. The engine-side download is always
/// cancelled — the Bun app process performs the actual download itself.
/// Returns TRUE (handled) so WebKit does not fall back to its own destination
/// policy for a download that is about to die anyway.
fn cbDecideDestination(obj: *gobject.Object, suggested: ?[*:0]const u8, _: ?*anyopaque) callconv(.c) c_int {
    const dl: *anyopaque = @ptrCast(obj);
    const cancel = ext.download_cancel orelse return 0;
    // The signal only lends the download. Cancelling drops WebKit's own
    // reference from inside the completion, so hold one across both the emit
    // and the cancel rather than racing the teardown.
    _ = gobject.Object.ref(obj);
    defer gobject.Object.unref(obj);

    if (downloadNodeId(dl)) |node_id| {
        if (ext.download_get_request) |get_request| {
            if (ext.uri_request_get_uri) |get_uri| {
                if (get_request(dl)) |request| {
                    if (get_uri(request)) |uri| {
                        var payload: std.json.ObjectMap = .empty;
                        defer payload.deinit(alloc);
                        objPutStr(&payload, "url", std.mem.span(uri));
                        if (downloadSuggestedFilename(dl, suggested)) |name| objPutStr(&payload, "suggestedFilename", name);
                        emitObject(node_id, "downloadRequested", payload);
                    }
                }
            }
        }
    }
    cancel(dl);
    return 1;
}

/// The node id of the view that started `dl`. Null for a download with no
/// originating view (`webkit_download_get_web_view` answers NULL for
/// context-level downloads) — those are still cancelled, just not reported.
fn downloadNodeId(dl: *anyopaque) ?u32 {
    const get_view = ext.download_get_web_view orelse return null;
    const view = get_view(dl) orelse return null;
    return widgetNodeId(@ptrCast(@alignCast(view)));
}

/// `suggestedFilename` per docs/webview.md: the engine's own suggestion (the
/// `decide-destination` argument), falling back to the Content-Disposition
/// name off the download's response. Never
/// `webkit_download_get_destination` — that is the local path WebKit would
/// write to, which is unset for a download we cancel.
fn downloadSuggestedFilename(dl: *anyopaque, suggested: ?[*:0]const u8) ?[]const u8 {
    if (suggested) |s| {
        const span = std.mem.span(s);
        if (span.len > 0) return span;
    }
    const get_response = ext.download_get_response orelse return null;
    const get_name = ext.uri_response_get_suggested_filename orelse return null;
    const response = get_response(dl) orelse return null;
    const name = get_name(response) orelse return null;
    const span = std.mem.span(name);
    return if (span.len > 0) span else null;
}

// ============================================================================
// Automation surface (webviewInfo / webviewEval / waitFor page predicates)
//
// These answer the automation socket directly and never route through the Bun
// child: a drive must be able to read page state from an app that forwards no
// events. Everything here runs on the UI thread inside one marshaled
// semantic_action call, so no locking is needed — but WebKit's evaluation is
// asynchronous, hence the start/poll pair rather than a blocking call.
// ============================================================================

/// Live page state. Strings are borrowed from the engine and are only valid
/// for the duration of the call.
pub const Info = struct {
    url: ?[]const u8,
    title: ?[]const u8,
    loading: bool,
    can_go_back: bool,
    can_go_forward: bool,
};

pub fn info(widget: *gtk.Widget) ?Info {
    const a = api orelse return null;
    if (!isReal(widget)) return null;
    const v: *anyopaque = @ptrCast(widget);
    const uri = a.web_view_get_uri(v);
    const title = a.web_view_get_title(v);
    return .{
        .url = if (uri) |u| std.mem.span(u) else null,
        .title = if (title) |t| std.mem.span(t) else null,
        .loading = if (ext.is_loading) |f| f(v) != 0 else false,
        .can_go_back = a.web_view_can_go_back(v) != 0,
        .can_go_forward = a.web_view_can_go_forward(v) != 0,
    };
}

/// One in-flight or settled automation eval. Kept in a global map keyed by id
/// rather than hung off the view, so a view destroyed mid-eval cannot leave
/// the completion callback writing through a dangling handle.
const PendingEval = struct {
    done: bool = false,
    ok: bool = false,
    value: ?[]u8 = null,
    err: ?[]u8 = null,
};

var pending_evals: std.AutoHashMapUnmanaged(u64, *PendingEval) = .empty;
var next_eval_id: u64 = 1;

const AutoEvalCtx = struct { id: u64 };

/// Starts an evaluation and returns its id, or null when the engine cannot
/// evaluate (no webkitgtk, or the symbol is missing). `world` empty/null means
/// the page's own world, matching `executeJavaScript`.
pub fn evalStart(widget: *gtk.Widget, code: []const u8, world: ?[]const u8) ?u64 {
    const eval = ext.evaluate_javascript orelse return null;
    if (!isReal(widget)) return null;
    const v: *anyopaque = @ptrCast(widget);

    const entry = alloc.create(PendingEval) catch return null;
    entry.* = .{};
    const id = next_eval_id;
    pending_evals.put(alloc, id, entry) catch {
        alloc.destroy(entry);
        return null;
    };
    next_eval_id += 1;

    const ctx = alloc.create(AutoEvalCtx) catch return null;
    ctx.* = .{ .id = id };
    const code_z = alloc.dupeZ(u8, code) catch {
        alloc.destroy(ctx);
        return null;
    };
    defer alloc.free(code_z); // WebKit copies the script synchronously before queuing
    const world_z = dupWorldZ(world);
    defer if (world_z) |w| alloc.free(w);
    eval(v, code_z.ptr, -1, if (world_z) |w| w.ptr else null, null, null, &cbAutoEvalReady, ctx);
    return id;
}

fn cbAutoEvalReady(source: ?*anyopaque, res: ?*anyopaque, user_data: ?*anyopaque) callconv(.c) void {
    const ctx: *AutoEvalCtx = @ptrCast(@alignCast(user_data orelse return));
    defer alloc.destroy(ctx);
    const entry = pending_evals.get(ctx.id) orelse return;
    entry.done = true;

    const finish = ext.evaluate_javascript_finish orelse {
        entry.err = alloc.dupe(u8, "webkit_web_view_evaluate_javascript_finish unavailable") catch null;
        return;
    };
    var err: ?*glib.Error = null;
    const value = finish(source orelse return, res, &err);
    if (value) |jsc_value| {
        defer gobject.Object.unref(@ptrCast(@alignCast(jsc_value)));
        entry.ok = true;
        if (ext.jsc_value_to_string) |to_str| {
            if (to_str(jsc_value)) |cstr| {
                defer glib.free(cstr);
                entry.value = alloc.dupe(u8, std.mem.span(cstr)) catch null;
            }
        }
        return;
    }
    if (err) |e| {
        defer e.free();
        const msg: []const u8 = if (e.f_message) |m| std.mem.span(m) else "unknown error";
        entry.err = alloc.dupe(u8, msg) catch null;
    } else {
        entry.err = alloc.dupe(u8, "unknown error") catch null;
    }
}

/// Snapshot of an eval by id. Null for an id that was never issued (or has
/// already been taken); `done` false means it is still running.
pub const EvalState = struct { done: bool, ok: bool, value: ?[]const u8, err: ?[]const u8 };

pub fn evalPoll(id: u64) ?EvalState {
    const entry = pending_evals.get(id) orelse return null;
    return .{ .done = entry.done, .ok = entry.ok, .value = entry.value, .err = entry.err };
}

/// Drops a settled eval. A still-running eval is NOT released — its completion
/// callback would then write through a freed entry — it is simply orphaned and
/// reaped by its own callback path on the next `evalRelease` after it settles.
pub fn evalRelease(id: u64) void {
    const entry = pending_evals.get(id) orelse return;
    if (!entry.done) return;
    _ = pending_evals.remove(id);
    if (entry.value) |v| alloc.free(v);
    if (entry.err) |e| alloc.free(e);
    alloc.destroy(entry);
}

/// The `pageTextContains` waitFor predicate's backing cache: one entry per
/// webview NODE (not widget pointer, so a destroyed view cannot be
/// dereferenced from the completion callback). Reading it kicks a refresh at
/// most once per `page_text_interval_us`, which is what keeps a ~50ms waitFor
/// tick from injecting JavaScript fifty times a second.
const page_text_interval_us: i64 = 250_000;
const PageText = struct { text: ?[]u8 = null, stamp_us: i64 = 0, in_flight: bool = false };
var page_texts: std.AutoHashMapUnmanaged(u32, PageText) = .empty;

const PageTextCtx = struct { node_id: u32 };

/// Last known `document.body.innerText`, and a refresh if the cache is stale.
/// Null until the first probe answers, so a predicate simply does not match
/// yet — the waitFor tick keeps calling.
pub fn pageText(widget: *gtk.Widget) ?[]const u8 {
    const node_id = widgetNodeId(widget) orelse return null;
    const gop = page_texts.getOrPut(alloc, node_id) catch return null;
    if (!gop.found_existing) gop.value_ptr.* = .{};
    const entry = gop.value_ptr;
    const now = glib.getMonotonicTime();
    if (!entry.in_flight and now - entry.stamp_us >= page_text_interval_us) {
        if (startPageTextProbe(widget, node_id)) entry.in_flight = true;
    }
    return entry.text;
}

fn startPageTextProbe(widget: *gtk.Widget, node_id: u32) bool {
    const eval = ext.evaluate_javascript orelse return false;
    if (!isReal(widget)) return false;
    const ctx = alloc.create(PageTextCtx) catch return false;
    ctx.* = .{ .node_id = node_id };
    const code = "document.body ? document.body.innerText : \"\"";
    eval(@ptrCast(widget), code, -1, null, null, null, &cbPageTextReady, ctx);
    return true;
}

fn cbPageTextReady(source: ?*anyopaque, res: ?*anyopaque, user_data: ?*anyopaque) callconv(.c) void {
    const ctx: *PageTextCtx = @ptrCast(@alignCast(user_data orelse return));
    defer alloc.destroy(ctx);
    const entry = page_texts.getPtr(ctx.node_id) orelse return;
    entry.in_flight = false;
    entry.stamp_us = glib.getMonotonicTime();

    const finish = ext.evaluate_javascript_finish orelse return;
    var err: ?*glib.Error = null;
    const value = finish(source orelse return, res, &err);
    if (err) |e| e.free();
    const jsc_value = value orelse return;
    defer gobject.Object.unref(@ptrCast(@alignCast(jsc_value)));
    const to_str = ext.jsc_value_to_string orelse return;
    const cstr = to_str(jsc_value) orelse return;
    defer glib.free(cstr);
    const next = alloc.dupe(u8, std.mem.span(cstr)) catch return;
    if (entry.text) |old| alloc.free(old);
    entry.text = next;
}
