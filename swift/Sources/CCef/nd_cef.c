#include "nd_cef.h"

#include <dlfcn.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// MARK: - Runtime loading

static struct {
  int (*execute_process)(const cef_main_args_t *, cef_app_t *, void *);
  int (*initialize)(const cef_main_args_t *, const cef_settings_t *, cef_app_t *, void *);
  void (*shutdown)(void);
  void (*run_message_loop)(void);
  void (*quit_message_loop)(void);
  int (*create_browser)(const cef_window_info_t *,
                        cef_client_t *,
                        const cef_string_t *,
                        const cef_browser_settings_t *,
                        cef_dictionary_value_t *,
                        cef_request_context_t *);
  const char *(*api_hash)(int, int);
  int (*string_utf8_to_utf16)(const char *, size_t, cef_string_utf16_t *);
  void (*string_userfree_free)(cef_string_userfree_utf16_t);
  size_t (*string_list_size)(cef_string_list_t);
  cef_string_list_t (*string_list_alloc)(void);
  void (*string_list_append)(cef_string_list_t, const cef_string_t *);
  void (*string_list_free)(cef_string_list_t);
  int (*string_list_value)(cef_string_list_t, size_t, cef_string_t *);
  cef_dictionary_value_t *(*dict_create)(void);
  cef_request_context_t *(*request_context_create)(const cef_request_context_settings_t *,
                                                   cef_request_context_handler_t *);
  int (*register_scheme_handler_factory)(const cef_string_t *,
                                         const cef_string_t *,
                                         cef_scheme_handler_factory_t *);
} g;

static void *g_handle = NULL;
static char g_error[512];

const char *nd_cef_load_error(void) {
  return g_error[0] ? g_error : NULL;
}

int nd_cef_is_loaded(void) {
  return g_handle != NULL;
}

static void *bind_symbol(const char *name) {
  void *sym = dlsym(g_handle, name);
  if (!sym) {
    snprintf(g_error, sizeof(g_error), "dlsym %s: %s", name, dlerror());
  }
  return sym;
}

int nd_cef_load(const char *framework_binary_path) {
  if (g_handle) {
    return 1;
  }
  g_error[0] = '\0';
  if (!framework_binary_path || !framework_binary_path[0]) {
    snprintf(g_error, sizeof(g_error), "no framework path");
    return 0;
  }
  // RTLD_FIRST keeps a symbol miss from falling through to another image and
  // resolving against something that is not CEF.
  g_handle = dlopen(framework_binary_path, RTLD_LAZY | RTLD_LOCAL | RTLD_FIRST);
  if (!g_handle) {
    snprintf(g_error, sizeof(g_error), "dlopen %s: %s", framework_binary_path, dlerror());
    return 0;
  }

  g.api_hash = bind_symbol("cef_api_hash");
  g.execute_process = bind_symbol("cef_execute_process");
  g.initialize = bind_symbol("cef_initialize");
  g.shutdown = bind_symbol("cef_shutdown");
  g.run_message_loop = bind_symbol("cef_run_message_loop");
  g.quit_message_loop = bind_symbol("cef_quit_message_loop");
  g.create_browser = bind_symbol("cef_browser_host_create_browser");
  g.string_utf8_to_utf16 = bind_symbol("cef_string_utf8_to_utf16");
  g.string_userfree_free = bind_symbol("cef_string_userfree_utf16_free");
  g.string_list_size = bind_symbol("cef_string_list_size");
  g.string_list_value = bind_symbol("cef_string_list_value");
  g.string_list_alloc = bind_symbol("cef_string_list_alloc");
  g.string_list_append = bind_symbol("cef_string_list_append");
  g.string_list_free = bind_symbol("cef_string_list_free");
  g.dict_create = bind_symbol("cef_dictionary_value_create");
  g.request_context_create = bind_symbol("cef_request_context_create_context");
  g.register_scheme_handler_factory = bind_symbol("cef_register_scheme_handler_factory");

  if (!g.api_hash || !g.execute_process || !g.initialize || !g.shutdown ||
      !g.run_message_loop || !g.quit_message_loop || !g.create_browser ||
      !g.string_utf8_to_utf16) {
    dlclose(g_handle);
    g_handle = NULL;
    memset(&g, 0, sizeof(g));
    return 0;
  }
  return 1;
}

// MARK: - Entry points

int nd_cef_execute_process(const cef_main_args_t *args,
                           cef_app_t *application,
                           void *windows_sandbox_info) {
  return g.execute_process ? g.execute_process(args, application, windows_sandbox_info) : 0;
}

int nd_cef_initialize(const cef_main_args_t *args,
                      const cef_settings_t *settings,
                      cef_app_t *application,
                      void *windows_sandbox_info) {
  return g.initialize ? g.initialize(args, settings, application, windows_sandbox_info) : 0;
}

void nd_cef_shutdown(void) {
  if (g.shutdown) {
    g.shutdown();
  }
}

void nd_cef_run_message_loop(void) {
  if (g.run_message_loop) {
    g.run_message_loop();
  }
}

void nd_cef_quit_message_loop(void) {
  if (g.quit_message_loop) {
    g.quit_message_loop();
  }
}

int nd_cef_create_browser(const cef_window_info_t *window_info,
                          cef_client_t *client,
                          const cef_string_t *url,
                          const cef_browser_settings_t *settings,
                          cef_dictionary_value_t *extra_info,
                          cef_request_context_t *request_context) {
  return g.create_browser ? g.create_browser(window_info, client, url, settings,
                                             extra_info, request_context)
                          : 0;
}

const char *nd_cef_api_hash(int version, int entry) {
  return g.api_hash ? g.api_hash(version, entry) : NULL;
}

int nd_cef_compiled_api_version(void) {
  return CEF_API_VERSION;
}

const char *nd_cef_compiled_api_hash(void) {
  return CEF_API_HASH_PLATFORM;
}

int nd_cef_string_set(const char *src, size_t src_len, cef_string_t *out) {
  if (!g.string_utf8_to_utf16 || !out) {
    return 0;
  }
  return g.string_utf8_to_utf16(src, src_len, out);
}

void nd_cef_string_clear(cef_string_t *value) {
  if (!value) {
    return;
  }
  if (value->dtor && value->str) {
    value->dtor(value->str);
  }
  value->str = NULL;
  value->length = 0;
  value->dtor = NULL;
}

void nd_cef_string_free(cef_string_userfree_t value) {
  if (value && g.string_userfree_free) {
    g.string_userfree_free(value);
  }
}

size_t nd_cef_string_list_count(cef_string_list_t list) {
  return (list && g.string_list_size) ? g.string_list_size(list) : 0;
}

int nd_cef_string_list_at(cef_string_list_t list, size_t index, cef_string_t *out) {
  return (list && out && g.string_list_value) ? g.string_list_value(list, index, out) : 0;
}

cef_string_list_t nd_cef_string_list_alloc(void) {
  return g.string_list_alloc ? g.string_list_alloc() : NULL;
}

void nd_cef_string_list_append(cef_string_list_t list, const cef_string_t *value) {
  if (list && value && g.string_list_append) {
    g.string_list_append(list, value);
  }
}

void nd_cef_string_list_free(cef_string_list_t list) {
  if (list && g.string_list_free) {
    g.string_list_free(list);
  }
}

cef_dictionary_value_t *nd_cef_dict_create(void) {
  return g.dict_create ? g.dict_create() : NULL;
}

cef_request_context_t *nd_cef_request_context_create(
    const cef_request_context_settings_t *settings,
    cef_request_context_handler_t *handler) {
  return g.request_context_create ? g.request_context_create(settings, handler) : NULL;
}

int nd_cef_register_scheme_handler_factory(const cef_string_t *scheme_name,
                                           const cef_string_t *domain_name,
                                           cef_scheme_handler_factory_t *factory) {
  return g.register_scheme_handler_factory
             ? g.register_scheme_handler_factory(scheme_name, domain_name, factory)
             : 0;
}

// MARK: - Refcounting

// Sits immediately before the CEF struct. Its 32-byte size keeps the struct
// itself on malloc's alignment, which matters because CEF reads it as a C
// aggregate.
typedef struct {
  _Atomic int32_t refs;
  int32_t padding;
  void *owner;
  nd_cef_owner_release_fn on_zero;
  uint64_t magic;
} nd_cef_ctl;

#define ND_CEF_CTL_MAGIC 0x6e64636566726566ULL

static nd_cef_ctl *ctl_of(void *obj) {
  if (!obj) {
    return NULL;
  }
  nd_cef_ctl *ctl = (nd_cef_ctl *)((char *)obj - sizeof(nd_cef_ctl));
  return ctl->magic == ND_CEF_CTL_MAGIC ? ctl : NULL;
}

static void ref_add(cef_base_ref_counted_t *self) {
  nd_cef_ctl *ctl = ctl_of(self);
  if (ctl) {
    atomic_fetch_add_explicit(&ctl->refs, 1, memory_order_relaxed);
  }
}

static int ref_release(cef_base_ref_counted_t *self) {
  nd_cef_ctl *ctl = ctl_of(self);
  if (!ctl) {
    return 0;
  }
  if (atomic_fetch_sub_explicit(&ctl->refs, 1, memory_order_acq_rel) != 1) {
    return 0;
  }
  if (ctl->on_zero) {
    ctl->on_zero(ctl->owner);
  }
  ctl->magic = 0;
  free(ctl);
  return 1;
}

static int ref_has_one(cef_base_ref_counted_t *self) {
  nd_cef_ctl *ctl = ctl_of(self);
  return ctl && atomic_load_explicit(&ctl->refs, memory_order_acquire) == 1;
}

static int ref_has_at_least_one(cef_base_ref_counted_t *self) {
  nd_cef_ctl *ctl = ctl_of(self);
  return ctl && atomic_load_explicit(&ctl->refs, memory_order_acquire) >= 1;
}

void *nd_cef_ref_alloc(size_t struct_size, void *owner, nd_cef_owner_release_fn on_zero) {
  if (struct_size < sizeof(cef_base_ref_counted_t)) {
    return NULL;
  }
  nd_cef_ctl *ctl = (nd_cef_ctl *)calloc(1, sizeof(nd_cef_ctl) + struct_size);
  if (!ctl) {
    return NULL;
  }
  atomic_store_explicit(&ctl->refs, 1, memory_order_relaxed);
  ctl->owner = owner;
  ctl->on_zero = on_zero;
  ctl->magic = ND_CEF_CTL_MAGIC;

  cef_base_ref_counted_t *base = (cef_base_ref_counted_t *)(ctl + 1);
  base->size = struct_size;
  base->add_ref = ref_add;
  base->release = ref_release;
  base->has_one_ref = ref_has_one;
  base->has_at_least_one_ref = ref_has_at_least_one;
  return base;
}

void *nd_cef_ref_owner(void *obj) {
  nd_cef_ctl *ctl = ctl_of(obj);
  return ctl ? ctl->owner : NULL;
}

void nd_cef_ref_add(void *obj) {
  if (!obj) {
    return;
  }
  cef_base_ref_counted_t *base = (cef_base_ref_counted_t *)obj;
  if (base->add_ref) {
    base->add_ref(base);
  }
}

void nd_cef_ref_release(void *obj) {
  if (!obj) {
    return;
  }
  cef_base_ref_counted_t *base = (cef_base_ref_counted_t *)obj;
  if (base->release) {
    base->release(base);
  }
}
