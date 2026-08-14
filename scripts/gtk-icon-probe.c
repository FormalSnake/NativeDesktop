/* Capture-environment guard for scripts/headless-docs-shots.sh.
 *
 * Asks GTK itself which icon theme it resolved and whether each named glyph
 * actually rasterizes. Two failures a shell-side file-existence check cannot
 * see, both of which shipped wrong pixels silently:
 *   - the settings portal on the live session bus handing GTK the desktop's
 *     own icon theme, whatever settings.ini and GSettings say;
 *   - a theme that HAS the icon file but whose symbolic SVG renders empty
 *     (gtk_icon_theme_has_icon still answers 1, so a lookup-only probe passes
 *     while the dropdown arrow is invisible).
 *
 * Usage: gtk-icon-probe <expected-theme> <icon-name>...
 * Needs a display, so run it after the compositor is up.
 */
#include <gtk/gtk.h>

/* Non-transparent pixels in a 16x16 render of the icon. */
static int render_ink(GtkIconPaintable *icon) {
  GtkSnapshot *snapshot = gtk_snapshot_new();
  gdk_paintable_snapshot(GDK_PAINTABLE(icon), snapshot, 16, 16);
  GskRenderNode *node = gtk_snapshot_free_to_node(snapshot);
  if (!node) return 0;
  cairo_surface_t *surface = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, 16, 16);
  cairo_t *cr = cairo_create(surface);
  gsk_render_node_draw(node, cr);
  cairo_destroy(cr);
  cairo_surface_flush(surface);
  const unsigned char *data = cairo_image_surface_get_data(surface);
  int stride = cairo_image_surface_get_stride(surface);
  int ink = 0;
  for (int y = 0; y < 16; y++)
    for (int x = 0; x < 16; x++)
      if (data[y * stride + x * 4 + 3] > 8) ink++;
  cairo_surface_destroy(surface);
  gsk_render_node_unref(node);
  return ink;
}

static gboolean check(GtkIconTheme *theme, const char *expected, const char *name) {
  GtkIconPaintable *icon =
      gtk_icon_theme_lookup_icon(theme, name, NULL, 16, 1, GTK_TEXT_DIR_LTR, 0);
  if (!icon) {
    g_printerr("FAIL: icon theme resolved no paintable for %s\n", name);
    return FALSE;
  }
  char *path = NULL;
  GFile *file = gtk_icon_paintable_get_file(icon);
  if (file) {
    path = g_file_get_path(file);
    g_object_unref(file);
  }
  int ink = render_ink(icon);
  g_object_unref(icon);

  gboolean ok = TRUE;
  char *needle = g_strdup_printf("/%s/", expected);
  if (!path || !strstr(path, needle)) {
    g_printerr("FAIL: %s came from %s, not the %s theme's own files\n", name,
               path ? path : "a built-in gresource", expected);
    ok = FALSE;
  }
  g_free(needle);
  if (ink == 0) {
    g_printerr("FAIL: %s rendered 0 ink pixels (blank glyph at %s)\n", name,
               path ? path : "(gresource)");
    ok = FALSE;
  }
  g_free(path);
  return ok;
}

int main(int argc, char **argv) {
  if (argc < 3) {
    g_printerr("usage: gtk-icon-probe <expected-theme> <icon-name>...\n");
    return 2;
  }
  const char *expected = argv[1];
  gtk_init();
  GdkDisplay *display = gdk_display_get_default();
  if (!display) {
    g_printerr("FAIL: no display (start the compositor before the probe)\n");
    return 1;
  }
  GtkIconTheme *theme = gtk_icon_theme_get_for_display(display);
  const char *resolved = gtk_icon_theme_get_theme_name(theme);
  if (g_strcmp0(resolved, expected) != 0) {
    g_printerr("FAIL: GTK resolved icon theme \"%s\", expected \"%s\" — the live "
               "session's settings are leaking in (see DBUS_SESSION_BUS_ADDRESS)\n",
               resolved, expected);
    return 1;
  }
  gboolean ok = TRUE;
  for (int i = 2; i < argc; i++) ok = check(theme, expected, argv[i]) && ok;
  if (!ok) return 1;
  g_print("docs-shots icons: %s, %d glyphs render\n", resolved, argc - 2);
  return 0;
}
