/* include/nd_plugin.h — the nd_plugin_v1 native-plugin ABI (spec §9, D12).
   A native plugin is a C-ABI shared library exporting nd_plugin_entry(), which
   returns a pointer to a statically-lived nd_plugin_v1. Capability strings are
   checked against the NDP-dispatch ACL; a command runs only if its plugin's
   declared permission is granted for the window. Opt-in: nothing loads a
   plugin unless the embedder calls nd_load_plugin (include/nd.h). */
#ifndef ND_PLUGIN_H
#define ND_PLUGIN_H
#include <stdint.h>
#include <stddef.h>

/* v2 (append-only): adds register_view + nd_view_impl so a plugin can register
   its OWN native widget, hosted by the backend like the builtin <terminal>/
   <webview>. v1 plugins keep loading unchanged — the core accepts abi_version
   1 or 2, and every new field is APPENDED (v1 code that reads only the original
   fields stays layout-valid). */
#define ND_PLUGIN_ABI_VERSION 2

typedef struct nd_plugin_registry nd_plugin_registry;

/* A plugin command handler: receives the arg JSON (NUL-terminated) and writes
   a malloc'd result JSON to *result_out (freed by the core via nd_free).
   Returns 0 ok, negative JSON-RPC-style code on error. */
typedef int32_t (*nd_command_fn)(const char* arg_json, char** result_out);

/* A native-view factory a plugin registers under a `view_kind` string. The
   handle each fn returns/takes is the backend-native widget as an opaque
   pointer (GtkWidget* on GTK, NSView* on AppKit) — the core never dereferences
   it; it flows through append_child/apply_style/unparent by parent kind alone.
   `props_json`/`arg_json` are NUL-terminated JSON. Native views are inherently
   backend-specific: a module ships a GTK impl and/or an AppKit impl. */
typedef struct nd_view_impl {
  void* (*create)(const char* props_json);
  void  (*apply_props)(void* view, const char* props_json);
  void  (*command)(void* view, const char* command, const char* arg_json);
  void  (*destroy)(void* view);
} nd_view_impl;

/* Host callbacks a plugin uses from init(). register_command associates a
   command name (reachable over NDP as {"type":"pluginCommand","name",...})
   with a handler; the core namespaces it as plugin:<plugin-name>.<command>.
   register_view (v2, appended) associates a `view_kind` string with a native-
   view factory the generic <nativeView> widget routes to. The core copies the
   nd_view_impl by value, so it may live on the plugin's stack in init(). */
struct nd_plugin_registry {
  void* host;   /* opaque core handle; pass back to the callbacks */
  void (*register_command)(nd_plugin_registry*, const char* command, nd_command_fn);
  void (*register_view)(nd_plugin_registry*, const char* view_kind, const nd_view_impl*);
};

typedef struct nd_plugin_v1 {
  uint32_t abi_version;         /* 1 or ND_PLUGIN_ABI_VERSION (2); a v1 plugin declares 1 and is loaded unchanged */
  const char* name;             /* plugin identity, e.g. "hello" */
  const char* const* capabilities; /* NULL-terminated permission strings, e.g. {"plugin:hello.greet", NULL} */
  int32_t (*init)(nd_plugin_registry*);  /* register commands/widgets; 0 ok */
  void (*deinit)(void);
} nd_plugin_v1;

/* Every plugin shared library exports this symbol. */
typedef const nd_plugin_v1* (*nd_plugin_entry_fn)(void);

#endif /* ND_PLUGIN_H */
