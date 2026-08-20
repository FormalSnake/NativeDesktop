// The CEF C API, translated from the binary distribution's own headers at
// build time. Nothing here is linked: `libcef.so` is dlopen'd (loader.zig) and
// every entry point is a symbol lookup, so a build with `-Dcef-dist=` still
// runs on a machine that has no CEF at all.
//
// CEF_API_VERSION is a layout pin, not a hint: struct sizes and field order in
// these headers change with it, and `cef_api_hash(version, 0)` compared against
// the loaded library is the only thing standing between a version skew and
// silent memory corruption. Keep it equal to `api_version` below and to what
// loader.zig checks.
pub const api_version: c_int = 15101;

pub const c = @cImport({
    @cDefine("CEF_API_VERSION", "15101");
    @cInclude("include/capi/cef_app_capi.h");
    @cInclude("include/capi/cef_browser_capi.h");
    @cInclude("include/capi/cef_client_capi.h");
    @cInclude("include/capi/cef_display_handler_capi.h");
    @cInclude("include/capi/cef_life_span_handler_capi.h");
    @cInclude("include/capi/cef_load_handler_capi.h");
    @cInclude("include/capi/cef_devtools_message_observer_capi.h");
    @cInclude("include/capi/cef_registration_capi.h");
    @cInclude("include/capi/cef_task_capi.h");
    @cInclude("include/capi/cef_parser_capi.h");
    @cInclude("include/capi/cef_values_capi.h");
    @cInclude("include/capi/cef_request_context_capi.h");
    @cInclude("include/capi/cef_find_handler_capi.h");
    @cInclude("include/capi/cef_download_handler_capi.h");
    @cInclude("include/capi/cef_jsdialog_handler_capi.h");
    @cInclude("include/capi/cef_scheme_capi.h");
    @cInclude("include/capi/cef_resource_handler_capi.h");
    @cInclude("include/capi/cef_command_line_capi.h");
    @cInclude("include/capi/cef_browser_process_handler_capi.h");
    @cInclude("include/capi/cef_context_menu_handler_capi.h");
    @cInclude("include/capi/cef_menu_model_capi.h");
    @cInclude("include/cef_api_hash.h");
});
