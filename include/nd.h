/* include/nd.h — the ONLY public surface of libnd. Hand-written; kept in
   lockstep with src/abi.zig by compile-time @sizeOf asserts. */
#ifndef ND_H
#define ND_H
#include <stdint.h>
#include <stdbool.h>

typedef void* nd_widget;              /* opaque backend handle (GtkWidget* / NSView*) */
typedef struct nd_context nd_context; /* opaque core instance */

/* Geometry in logical window-top-left space (matches getTree contract). */
typedef struct { int32_t x, y, w, h; } nd_rect;

/* The backend vtable: the embedder fills these; the core calls up through them.
   `props_json` / `arg_json` are NUL-terminated UTF-8 JSON (M6a-D2). All calls
   arrive on the embedder's UI thread (the core marshals). */
typedef struct nd_backend {
  /* structural + prop ops (mirror the Zig seam) */
  nd_widget (*create)(nd_context*, const char* kind, const char* props_json);
  void (*apply_props)(nd_context*, nd_widget, const char* kind, const char* props_json);
  void (*append_child)(nd_context*, nd_widget parent, const char* parent_kind,
                       nd_widget child, const char* attached_json);
  void (*insert_before)(nd_context*, nd_widget parent, const char* parent_kind,
                        nd_widget child, nd_widget before /* nullable */,
                        const char* attached_json);
  void (*remove_child)(nd_context*, nd_widget parent, const char* parent_kind, nd_widget child);
  void (*set_text)(nd_context*, nd_widget, const char* text);
  void (*set_visible)(nd_context*, nd_widget, bool visible);
  void (*apply_style)(nd_context*, nd_widget, uint32_t node_id, const char* style_json);
  void (*connect_events)(nd_context*, nd_widget, const char* kind, uint32_t node_id);
  bool (*has_parent)(nd_context*, nd_widget);
  void (*unparent)(nd_context*, nd_widget);
  nd_widget (*get_window)(nd_context*);

  /* embedder UI-thread marshal + host chrome (M6a Task 3): the core's
     commit-apply/child-exit/overlay paint call up through these instead of
     importing glib/gio directly. GTK fills marshal_async with
     g_main_context_invoke_full; the Mac shell fills it with
     dispatch_async_f. `show_overlay("")` (empty message) is the clear
     sentinel — the core calls it that way from its dev-mode Restart/respawn
     path instead of a dedicated clear-overlay vtable field. */
  void (*marshal_async)(nd_context*, void (*fn)(void*), void* data);
  void (*show_overlay)(nd_context*, const char* message);

  /* automation backend half (M6a-D3) */
  bool (*node_visible)(nd_context*, nd_widget);
  bool (*node_bounds)(nd_context*, nd_widget, nd_rect* out);   /* false = no bounds */
  bool (*snapshot)(nd_context*, const char* png_path);         /* in-process render */
  /* action: "click"|"setValue"|"type"|"scroll"; arg_json carries params.
     returns 0 ok, or a negative JSON-RPC-style code; err_json_out (nullable,
     caller frees via nd_free) gets a data object on failure. */
  int32_t (*semantic_action)(nd_context*, nd_widget, uint32_t node_id,
                             const char* action, const char* arg_json,
                             char** result_json_out, char** err_json_out);
} nd_backend;

/* lifecycle */
nd_context* nd_init(void);                                  /* create core, spawn nothing yet */
void nd_register_backend(nd_context*, const nd_backend*);   /* store the vtable */
int32_t nd_start_runtime(nd_context*);                      /* open NDP socket, spawn bun child */
int32_t nd_start_automation(nd_context*);                   /* open automation socket + thread */
void nd_free(void* p);                                      /* free a core-allocated string */

/* embedder -> core: a native event happened (button clicked, text changed, …).
   `payload_json` is a NUL-terminated JSON object or "{}". */
void nd_emit_event(nd_context*, uint32_t node_id, const char* name, const char* payload_json);

#endif /* ND_H */
