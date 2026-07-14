#ifndef ND_NATIVE_GTK_H
#define ND_NATIVE_GTK_H

#include <gtk/gtk.h>
#include "../include/nd_plugin.h"

/* Small C convenience layer for app-owned GTK plugins. The app still exports
   nd_plugin_entry and owns every widget/state allocation. */
typedef struct nd_gtk_view_state {
  nd_plugin_registry* registry;
  uint32_t node_id;
} nd_gtk_view_state;

static inline void nd_gtk_connect_state(nd_gtk_view_state* state, nd_plugin_registry* registry, uint32_t node_id) {
  state->registry = registry;
  state->node_id = node_id;
}

static inline void nd_gtk_emit(nd_gtk_view_state* state, const char* name, const char* payload_json) {
  if (state && state->registry && state->registry->emit_event)
    state->registry->emit_event(state->registry, state->node_id, name, payload_json ? payload_json : "{}");
}

#endif
