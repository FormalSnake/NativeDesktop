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

#define ND_PLUGIN_ABI_VERSION 1

typedef struct nd_plugin_registry nd_plugin_registry;

/* A plugin command handler: receives the arg JSON (NUL-terminated) and writes
   a malloc'd result JSON to *result_out (freed by the core via nd_free).
   Returns 0 ok, negative JSON-RPC-style code on error. */
typedef int32_t (*nd_command_fn)(const char* arg_json, char** result_out);

/* Host callbacks a plugin uses from init(). register_command associates a
   command name (reachable over NDP as {"type":"pluginCommand","name",...})
   with a handler; the core namespaces it as plugin:<plugin-name>.<command>. */
struct nd_plugin_registry {
  void* host;   /* opaque core handle; pass back to the callbacks */
  void (*register_command)(nd_plugin_registry*, const char* command, nd_command_fn);
};

typedef struct nd_plugin_v1 {
  uint32_t abi_version;         /* must == ND_PLUGIN_ABI_VERSION */
  const char* name;             /* plugin identity, e.g. "hello" */
  const char* const* capabilities; /* NULL-terminated permission strings, e.g. {"plugin:hello.greet", NULL} */
  int32_t (*init)(nd_plugin_registry*);  /* register commands/widgets; 0 ok */
  void (*deinit)(void);
} nd_plugin_v1;

/* Every plugin shared library exports this symbol. */
typedef const nd_plugin_v1* (*nd_plugin_entry_fn)(void);

#endif /* ND_PLUGIN_H */
