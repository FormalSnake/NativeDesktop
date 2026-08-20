// CEF's C API, plus the two pieces Swift cannot express itself: a runtime
// loader (the framework is dlopened, never linked, so an engine=system app
// carries no CEF bytes) and one refcount harness for the capi structs we hand
// back to the library.
//
// Every CEF entry point below is a forwarder onto a dlsym'd pointer, the same
// shape as the distribution's own libcef_dll_dylib.cc. Exporting the pointers
// directly would make them shared mutable state to Swift 6.
//
// The CEF headers themselves are not vendored. swift/Package.swift points the
// header search path at the extracted distribution (ND_CEF_ROOT, falling back
// to the dev cache) and defines CEF_API_VERSION there; when neither exists the
// target is dropped from the package and `#if canImport(CCef)` compiles the
// engine out.

#ifndef ND_CEF_H
#define ND_CEF_H

#include <stddef.h>
#include <stdint.h>

#include "include/capi/cef_app_capi.h"
#include "include/capi/cef_browser_capi.h"
#include "include/capi/cef_command_line_capi.h"
#include "include/capi/cef_client_capi.h"
#include "include/capi/cef_display_handler_capi.h"
#include "include/capi/cef_life_span_handler_capi.h"
#include "include/capi/cef_context_menu_handler_capi.h"
#include "include/capi/cef_cookie_capi.h"
#include "include/capi/cef_devtools_message_observer_capi.h"
#include "include/capi/cef_dialog_handler_capi.h"
#include "include/capi/cef_download_handler_capi.h"
#include "include/capi/cef_find_handler_capi.h"
#include "include/capi/cef_focus_handler_capi.h"
#include "include/capi/cef_jsdialog_handler_capi.h"
#include "include/capi/cef_load_handler_capi.h"
#include "include/capi/cef_registration_capi.h"
#include "include/capi/cef_request_context_capi.h"
#include "include/capi/cef_request_handler_capi.h"
#include "include/capi/cef_resource_request_handler_capi.h"
#include "include/capi/cef_resource_handler_capi.h"
#include "include/capi/cef_scheme_capi.h"
#include "include/capi/cef_values_capi.h"

#ifdef __cplusplus
extern "C" {
#endif

// MARK: - Runtime loading

/// dlopen the framework binary at |path| and bind the entry points below.
/// Returns 0 on failure, leaving the library unloaded. Safe to call twice: the
/// second call is a no-op reporting the first call's result.
int nd_cef_load(const char *framework_binary_path);

/// Whether nd_cef_load has succeeded in this process.
int nd_cef_is_loaded(void);

/// dlopen/dlsym text from the last failed nd_cef_load, or NULL.
const char *nd_cef_load_error(void);

// MARK: - Entry points
// Calling any of these before a successful nd_cef_load is a no-op returning 0.

int nd_cef_execute_process(const cef_main_args_t *args,
                           cef_app_t *application,
                           void *windows_sandbox_info);
int nd_cef_initialize(const cef_main_args_t *args,
                      const cef_settings_t *settings,
                      cef_app_t *application,
                      void *windows_sandbox_info);
void nd_cef_shutdown(void);
void nd_cef_run_message_loop(void);
void nd_cef_quit_message_loop(void);
int nd_cef_create_browser(const cef_window_info_t *window_info,
                          cef_client_t *client,
                          const cef_string_t *url,
                          const cef_browser_settings_t *settings,
                          cef_dictionary_value_t *extra_info,
                          cef_request_context_t *request_context);
const char *nd_cef_api_hash(int version, int entry);

/// CEF_API_VERSION and the platform hash these headers were compiled against.
/// The loaded framework must agree, or its structs are laid out differently
/// from the ones this process fills in.
int nd_cef_compiled_api_version(void);
const char *nd_cef_compiled_api_hash(void);

/// Fills |out| with a copy of the UTF-8 |src|, through CEF's own allocator so
/// the dtor CEF expects is in place. Returns 0 on failure.
int nd_cef_string_set(const char *src, size_t src_len, cef_string_t *out);

/// Frees a string filled by nd_cef_string_set.
void nd_cef_string_clear(cef_string_t *value);

/// Frees a cef_string_userfree_t the library returned.
void nd_cef_string_free(cef_string_userfree_t value);

/// cef_string_list_t, for the handlers that report or take lists of strings.
size_t nd_cef_string_list_count(cef_string_list_t list);
int nd_cef_string_list_at(cef_string_list_t list, size_t index, cef_string_t *out);
cef_string_list_t nd_cef_string_list_alloc(void);
void nd_cef_string_list_append(cef_string_list_t list, const cef_string_t *value);
void nd_cef_string_list_free(cef_string_list_t list);

/// An empty dictionary for DevTools method params.
cef_dictionary_value_t *nd_cef_dict_create(void);

/// One request context, which is what a `profile` resolves to.
cef_request_context_t *nd_cef_request_context_create(
    const cef_request_context_settings_t *settings,
    cef_request_context_handler_t *handler);

/// Process-wide scheme handler factory. |domain_name| may be empty.
int nd_cef_register_scheme_handler_factory(const cef_string_t *scheme_name,
                                           const cef_string_t *domain_name,
                                           cef_scheme_handler_factory_t *factory);

/// The process-wide cef_app_t, shared by the host and the helper because
/// on_register_custom_schemes has to answer identically in every process.
/// Pass |browser_process| as 1 in the host, 0 in a helper. The caller owns the
/// returned reference.
///
/// Custom schemes come from ND_CEF_SCHEMES (comma separated), which children
/// inherit through the environment. A scheme has to be declared before
/// cef_initialize to be standard, secure and CORS enabled, and no app code has
/// run by then, so the env var is the only channel that reaches every process.
cef_app_t *nd_cef_app_create(int browser_process);

// MARK: - Refcounting

/// Runs when an object's last reference goes, before its block is freed.
typedef void (*nd_cef_owner_release_fn)(void *owner);

/// One refcounted capi object: |struct_size| zeroed bytes with base.size and
/// the four base callbacks wired, preceded by a private control block. The
/// returned pointer IS the CEF struct, and it starts with one reference owned
/// by the caller. When the last reference goes, |on_zero| runs against |owner|
/// and the block is freed.
///
/// Every handler in the Swift engine comes from here. Open-coding add_ref and
/// release per handler is how capi integrations leak or double-free.
void *nd_cef_ref_alloc(size_t struct_size,
                       void *owner,
                       nd_cef_owner_release_fn on_zero);

/// The |owner| an object was allocated with, or NULL if |obj| did not come
/// from nd_cef_ref_alloc.
void *nd_cef_ref_owner(void *obj);

/// base.add_ref / base.release on any CEF object, ours or the library's.
void nd_cef_ref_add(void *obj);
void nd_cef_ref_release(void *obj);

#ifdef __cplusplus
}
#endif

#endif  // ND_CEF_H
