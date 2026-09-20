/* A settings portal that answers exactly one key, for scripts/headless-decoration.sh.
 *
 * The gate needs a desktop whose window-button layout it chose, and the real
 * xdg-desktop-portal reads the capture host's own dconf. This owns
 * org.freedesktop.portal.Desktop on the private bus the gate starts, so the
 * activatable real portal never comes up, and answers
 * org.gnome.desktop.wm.preferences/button-layout with the string it was given.
 *
 * Usage: portal-settings-stub <button-layout>
 * Prints ND_PORTAL_STUB_READY once it owns the name.
 */
#include <gio/gio.h>
#include <stdio.h>
#include <stdlib.h>

static const char *layout = ":";

static const char introspection[] =
    "<node><interface name='org.freedesktop.portal.Settings'>"
    "<method name='ReadOne'>"
    "<arg type='s' name='namespace' direction='in'/>"
    "<arg type='s' name='key' direction='in'/>"
    "<arg type='v' name='value' direction='out'/>"
    "</method>"
    "<method name='Read'>"
    "<arg type='s' name='namespace' direction='in'/>"
    "<arg type='s' name='key' direction='in'/>"
    "<arg type='v' name='value' direction='out'/>"
    "</method>"
    "</interface></node>";

static void on_call(GDBusConnection *conn, const char *sender, const char *path,
                    const char *iface, const char *method, GVariant *params,
                    GDBusMethodInvocation *inv, gpointer user_data) {
  (void)conn; (void)sender; (void)path; (void)iface; (void)user_data;
  const char *ns = NULL, *key = NULL;
  g_variant_get(params, "(&s&s)", &ns, &key);
  if (!g_str_equal(ns, "org.gnome.desktop.wm.preferences") ||
      !g_str_equal(key, "button-layout")) {
    g_dbus_method_invocation_return_dbus_error(
        inv, "org.freedesktop.portal.Error.NotFound", "no such key");
    return;
  }
  GVariant *value = g_variant_new_string(layout);
  /* Read predates ReadOne and wraps the value in one more variant. */
  if (g_str_equal(method, "Read")) value = g_variant_new_variant(value);
  g_dbus_method_invocation_return_value(inv, g_variant_new("(v)", value));
}

static const GDBusInterfaceVTable vtable = {on_call, NULL, NULL, {0}};

static void on_bus(GDBusConnection *conn, const char *name, gpointer user_data) {
  (void)name;
  GDBusNodeInfo *node = user_data;
  GError *err = NULL;
  g_dbus_connection_register_object(conn, "/org/freedesktop/portal/desktop",
                                    node->interfaces[0], &vtable, NULL, NULL, &err);
  if (err) {
    g_printerr("register failed: %s\n", err->message);
    exit(1);
  }
}

static void on_acquired(GDBusConnection *conn, const char *name, gpointer _u) {
  (void)conn; (void)name; (void)_u;
  g_print("ND_PORTAL_STUB_READY layout=%s\n", layout);
  fflush(stdout);
}

static void on_lost(GDBusConnection *conn, const char *name, gpointer _u) {
  (void)conn; (void)_u;
  g_printerr("lost the name %s\n", name);
  exit(1);
}

int main(int argc, char **argv) {
  if (argc > 1) layout = argv[1];
  GDBusNodeInfo *node = g_dbus_node_info_new_for_xml(introspection, NULL);
  g_bus_own_name(G_BUS_TYPE_SESSION, "org.freedesktop.portal.Desktop",
                 G_BUS_NAME_OWNER_FLAGS_NONE, on_bus, on_acquired, on_lost, node, NULL);
  g_main_loop_run(g_main_loop_new(NULL, FALSE));
  return 0;
}
