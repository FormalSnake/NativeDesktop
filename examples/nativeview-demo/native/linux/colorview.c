#include <gtk/gtk.h>
#include <stdlib.h>
#include <string.h>
#include "nd_native_gtk.h"

typedef struct {
  nd_gtk_view_state nd;
  GtkWidget* area;
  double r, g, b;
} ColorView;

static nd_plugin_registry* host_registry;

static void parse_color(ColorView* s, const char* json) {
  const char* p = json ? strchr(json, '#') : NULL;
  unsigned value = p ? (unsigned)strtoul(p + 1, NULL, 16) : 0x3b82f6;
  s->r = ((value >> 16) & 255) / 255.0; s->g = ((value >> 8) & 255) / 255.0; s->b = (value & 255) / 255.0;
}
static void draw(GtkDrawingArea* area, cairo_t* cr, int width, int height, gpointer data) {
  ColorView* s = data; (void)area;
  cairo_set_source_rgb(cr, s->r, s->g, s->b); cairo_rectangle(cr, 0, 0, width, height); cairo_fill(cr);
}
static void clicked(GtkGestureClick* gesture, int count, double x, double y, gpointer data) {
  ColorView* s = data; (void)gesture; (void)count;
  char payload[64];
  g_snprintf(payload, sizeof payload, "{\"source\":\"gtk\",\"x\":%d,\"y\":%d}", (int)x, (int)y);
  nd_gtk_emit(&s->nd, "pressed", payload);
}
static void* create_view(const char* props) {
  ColorView* s = calloc(1, sizeof(*s)); if (!s) return NULL;
  s->nd.registry = host_registry; parse_color(s, props);
  s->area = gtk_drawing_area_new(); gtk_drawing_area_set_content_width(GTK_DRAWING_AREA(s->area), 320); gtk_drawing_area_set_content_height(GTK_DRAWING_AREA(s->area), 200);
  g_object_set_data(G_OBJECT(s->area), "nd-color-state", s);
  gtk_drawing_area_set_draw_func(GTK_DRAWING_AREA(s->area), draw, s, NULL);
  GtkGesture* click = gtk_gesture_click_new(); g_signal_connect(click, "pressed", G_CALLBACK(clicked), s); gtk_widget_add_controller(s->area, GTK_EVENT_CONTROLLER(click));
  return s->area;
}
static void apply_props(void* view, const char* props) { ColorView* s = g_object_get_data(G_OBJECT(view), "nd-color-state"); if (s) { parse_color(s, props); gtk_widget_queue_draw(view); } }
static void connect_view(void* view, uint32_t node_id) { ColorView* s = g_object_get_data(G_OBJECT(view), "nd-color-state"); if (s) nd_gtk_connect_state(&s->nd, host_registry, node_id); }
static void command_view(void* view, const char* command, const char* arg) {
  (void)arg;
  ColorView* s = g_object_get_data(G_OBJECT(view), "nd-color-state");
  if (s && command && strcmp(command, "reset") == 0) {
    parse_color(s, "{\"color\":\"#3b82f6\"}");
    gtk_widget_queue_draw(view);
  }
}
static void destroy_view(void* view) { ColorView* s = g_object_get_data(G_OBJECT(view), "nd-color-state"); if (s) { g_object_set_data(G_OBJECT(view), "nd-color-state", NULL); free(s); } }

static nd_view_impl impl = { create_view, apply_props, command_view, destroy_view, connect_view };
static int32_t init(nd_plugin_registry* registry) { host_registry = registry; registry->register_view(registry, "app.colorview", &impl); return 0; }
static void deinit(void) {}
static const char* capabilities[] = { NULL };
static const nd_plugin_v1 plugin = { ND_PLUGIN_ABI_VERSION, "app-colorview", capabilities, init, deinit };
const nd_plugin_v1* nd_plugin_entry(void) { return &plugin; }
