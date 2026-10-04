#include "nd_cef.h"

#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
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
  cef_value_t *(*value_create)(void);
  cef_request_context_t *(*request_context_create)(const cef_request_context_settings_t *,
                                                   cef_request_context_handler_t *);
  int (*register_scheme_handler_factory)(const cef_string_t *,
                                         const cef_string_t *,
                                         cef_scheme_handler_factory_t *);
  cef_browser_view_t *(*browser_view_create)(cef_client_t *,
                                             const cef_string_t *,
                                             const cef_browser_settings_t *,
                                             cef_dictionary_value_t *,
                                             cef_request_context_t *,
                                             cef_browser_view_delegate_t *);
  cef_window_t *(*window_create_top_level)(cef_window_delegate_t *);
  int (*id_for_command_id_name)(const char *);
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
  g.value_create = bind_symbol("cef_value_create");
  g.request_context_create = bind_symbol("cef_request_context_create_context");
  g.register_scheme_handler_factory = bind_symbol("cef_register_scheme_handler_factory");
  g.browser_view_create = bind_symbol("cef_browser_view_create");
  g.window_create_top_level = bind_symbol("cef_window_create_top_level");
  g.id_for_command_id_name = bind_symbol("cef_id_for_command_id_name");

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

cef_browser_view_t *nd_cef_browser_view_create(cef_client_t *client,
                                               const cef_string_t *url,
                                               const cef_browser_settings_t *settings,
                                               cef_dictionary_value_t *extra_info,
                                               cef_request_context_t *request_context,
                                               cef_browser_view_delegate_t *delegate) {
  return g.browser_view_create ? g.browser_view_create(client, url, settings, extra_info,
                                                       request_context, delegate)
                               : NULL;
}

cef_window_t *nd_cef_window_create_top_level(cef_window_delegate_t *delegate) {
  return g.window_create_top_level ? g.window_create_top_level(delegate) : NULL;
}

int nd_cef_command_id(const char *name) {
  return (name && g.id_for_command_id_name) ? g.id_for_command_id_name(name) : -1;
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

cef_value_t *nd_cef_value_create(void) {
  return g.value_create ? g.value_create() : NULL;
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

// MARK: - Application

static void CEF_CALLBACK app_register_schemes(cef_app_t *self, cef_scheme_registrar_t *registrar) {
  (void)self;
  if (!registrar || !registrar->add_custom_scheme) {
    return;
  }
  // Content blocking's own scheme (src/adblock.zig `serve`), in every process
  // and for every app: the renderers fetch from it from inside pages whose CSP
  // would refuse anything else.
  cef_string_t adblock = {0};
  if (nd_cef_string_set("nd-adblock", strlen("nd-adblock"), &adblock)) {
    registrar->add_custom_scheme(registrar, &adblock,
                                 CEF_SCHEME_OPTION_STANDARD | CEF_SCHEME_OPTION_SECURE |
                                     CEF_SCHEME_OPTION_CORS_ENABLED | CEF_SCHEME_OPTION_CSP_BYPASSING);
    nd_cef_string_clear(&adblock);
  }
  const char *list = getenv("ND_CEF_SCHEMES");
  if (!list || !list[0]) {
    return;
  }
  const int options = CEF_SCHEME_OPTION_STANDARD | CEF_SCHEME_OPTION_SECURE |
                      CEF_SCHEME_OPTION_CORS_ENABLED | CEF_SCHEME_OPTION_FETCH_ENABLED;
  const char *cursor = list;
  while (*cursor) {
    const char *comma = strchr(cursor, ',');
    size_t len = comma ? (size_t)(comma - cursor) : strlen(cursor);
    if (len > 0 && len < 64) {
      char name[64];
      memcpy(name, cursor, len);
      name[len] = '\0';
      cef_string_t scheme = {0};
      if (nd_cef_string_set(name, len, &scheme)) {
        if (!registrar->add_custom_scheme(registrar, &scheme, options)) {
          fprintf(stderr, "ND_WARN cef scheme %s: the engine refused to register it\n", name);
        }
        nd_cef_string_clear(&scheme);
      }
    }
    if (!comma) {
      break;
    }
    cursor = comma + 1;
  }
}

// Chromium's --remote-debugging-pipe reads the browser target's protocol from
// fd 3 and writes it to fd 4, both fixed on POSIX
// (content/browser/devtools/devtools_agent_host_impl.cc), and holds them for
// the whole run. So the two slots are claimed before anything else in the
// process opens a descriptor, and only when both are free: a launcher that
// handed this process an fd 3 or 4 keeps it, and the pipe is simply absent.
static int browser_pipe_to_cef = -1;
static int browser_pipe_from_cef = -1;

static int fd_is_free(int fd) {
  return fcntl(fd, F_GETFD) == -1 && errno == EBADF;
}

static int move_to(int from, int to) {
  if (from == to) return 1;
  if (dup2(from, to) != to) return 0;
  close(from);
  return 1;
}

int nd_cef_reserve_browser_pipe(void) {
  if (browser_pipe_to_cef >= 0) return 1;
  if (!fd_is_free(3) || !fd_is_free(4)) return 0;
  int in[2], out[2];
  if (pipe(in) != 0) return 0;
  if (pipe(out) != 0) {
    close(in[0]);
    close(in[1]);
    return 0;
  }
  // pipe() takes the lowest free numbers, so in[0] may already sit on 3 and
  // out[1] on 4; the others are moved off 3 and 4 before those are filled.
  int host_write = fcntl(in[1], F_DUPFD_CLOEXEC, 5);
  int host_read = fcntl(out[0], F_DUPFD_CLOEXEC, 5);
  close(in[1]);
  close(out[0]);
  if (host_write < 0 || host_read < 0 || !move_to(in[0], 3) || !move_to(out[1], 4)) {
    if (host_write >= 0) close(host_write);
    if (host_read >= 0) close(host_read);
    close(3);
    close(4);
    return 0;
  }
  fcntl(3, F_SETFD, FD_CLOEXEC);
  fcntl(4, F_SETFD, FD_CLOEXEC);
  browser_pipe_to_cef = host_write;
  browser_pipe_from_cef = host_read;
  return 1;
}

int nd_cef_browser_pipe_write_fd(void) { return browser_pipe_to_cef; }
int nd_cef_browser_pipe_read_fd(void) { return browser_pipe_from_cef; }

static void append_switch(cef_command_line_t *command_line, const char *name) {
  cef_string_t value = {0};
  if (nd_cef_string_set(name, strlen(name), &value)) {
    command_line->append_switch(command_line, &value);
    nd_cef_string_clear(&value);
  }
}

// Joins whatever value the launch already carries for `switch_name`: Chromium
// reads one comma-separated switch, and a second append would replace the
// earlier list rather than add to it.
static void append_joined(cef_command_line_t *command_line, const char *switch_name, const char *value) {
  if (!value || !value[0] || !command_line->append_switch_with_value) {
    return;
  }
  cef_string_t name = {0};
  if (!nd_cef_string_set(switch_name, strlen(switch_name), &name)) {
    return;
  }
  char joined[8192];
  joined[0] = '\0';
  if (command_line->get_switch_value) {
    cef_string_userfree_t existing = command_line->get_switch_value(command_line, &name);
    if (existing) {
      size_t n = 0;
      for (size_t i = 0; i < existing->length && n + 4 < sizeof(joined); i++) {
        char16_t ch = existing->str[i];
        if (ch < 0x80) {
          joined[n++] = (char)ch;
        } else if (ch < 0x800) {
          joined[n++] = (char)(0xC0 | (ch >> 6));
          joined[n++] = (char)(0x80 | (ch & 0x3F));
        } else {
          joined[n++] = (char)(0xE0 | (ch >> 12));
          joined[n++] = (char)(0x80 | ((ch >> 6) & 0x3F));
          joined[n++] = (char)(0x80 | (ch & 0x3F));
        }
      }
      joined[n] = '\0';
      nd_cef_string_free(existing);
    }
  }
  size_t used = strlen(joined);
  if (used + strlen(value) + 2 >= sizeof(joined)) {
    nd_cef_string_clear(&name);
    return;
  }
  if (used > 0) {
    strcat(joined, ",");
  }
  strcat(joined, value);
  cef_string_t joined_value = {0};
  if (nd_cef_string_set(joined, strlen(joined), &joined_value)) {
    command_line->append_switch_with_value(command_line, &name, &joined_value);
    nd_cef_string_clear(&joined_value);
  }
  nd_cef_string_clear(&name);
}

static void CEF_CALLBACK app_command_line(cef_app_t *self,
                                          const cef_string_t *process_type,
                                          cef_command_line_t *command_line) {
  (void)self;
  // Browser process only: a child's command line is Chromium's to build.
  int is_browser = process_type == NULL || process_type->length == 0;
  if (is_browser && command_line && command_line->append_switch) {
    append_switch(command_line, "disable-popup-blocking");
    const char *style = getenv("ND_CEF_STYLE");
    if (style && strcmp(style, "chrome") == 0) {
      // Chrome style carries Chrome's own browser UI, and several of its
      // startup and print surfaces open a top-level window of their own. None
      // of them has a client callback to answer; the switch is the only hook.
      append_switch(command_line, "no-first-run");
      append_switch(command_line, "no-default-browser-check");
      append_switch(command_line, "disable-print-preview");
      append_joined(command_line, "load-extension", getenv("ND_CEF_FRAMEWORK_EXTENSION"));
      if (browser_pipe_to_cef >= 0) {
        append_switch(command_line, "remote-debugging-pipe");
        // The pipe alone turns on Blink's AutomationControlled, which is what
        // sets navigator.webdriver, and sites such as Google Search then
        // answer with a bot check. The pipe is the framework's own channel.
        append_joined(command_line, "disable-blink-features", "AutomationControlled");
      }
      // The floating video keeps the page's origin over the picture until the
      // user presses inside it or the site is trusted for media
      // (VideoOverlayWindowViews::UpdateControlsVisibility); the host's own
      // controls take every press, so the title would never leave. This
      // feature only short-circuits that trust check. Being a ForTesting
      // feature it can disappear in a CEF update, and an unknown feature name
      // is ignored silently: scripts/mac/pip-feature-check.sh fails if the
      // framework binary no longer carries the name.
      append_joined(command_line, "enable-features", "VideoPipForceTrustedForMediaPlaybackForTesting");
    }
    // Domain Reliability uploads network error samples to Google; nothing in
    // an embedded browser reads them.
    append_switch(command_line, "disable-domain-reliability");
    // Background services with no surface in an embedded browser: Cast device
    // discovery, Google's page hints and autofill form signatures, and
    // Translate, whose language detection otherwise runs on every page load.
    // The omnibox popups as WebUI preload two pages for every browser, and
    // every view here is a browser whose omnibox nobody sees; the Views popup
    // is only built when an omnibox opens one.
    append_joined(command_line, "disable-features", "MediaRouter,OptimizationHints,AutofillServerCommunication,Translate,WebUIOmniboxPopup,WebUIOmniboxFullPopup,WebUIOmniboxAimPopup");
    // Read by StartupBrowserCreator, which CEF skips at startup and a refused
    // relaunch (below) never reaches; this covers any other route into it.
    // Not --no-startup-window: on Linux it holds a keep-alive that stops
    // CefShutdown from returning.
    append_switch(command_line, "hide-crash-restore-bubble");
    // A test host on a throwaway profile keeps Chromium's cookie key out of
    // the login keychain. A host started with HOME pointed at a fixture dir
    // has no default keychain, and storing "Chromium Safe Storage" then puts
    // up a system "keychain cannot be found" dialog over the gate. The
    // ND_CEF_CACHE requirement keeps the mock key out of a real profile: its
    // existing cookies were encrypted with the keychain key.
    const char *automation = getenv("NATIVE_AUTOMATION");
    const char *cache = getenv("ND_CEF_CACHE");
    if (automation && strcmp(automation, "1") == 0 && cache && cache[0]) {
      append_switch(command_line, "use-mock-keychain");
    }
  }
  nd_cef_ref_release(command_line);
}

// Another launch on this root cache path. Chromium's process singleton has
// forwarded its command line here and will make that process's cef_initialize
// fail; left unanswered, Chrome's StartupBrowserCreator opens a "New Tab"
// browser window in this process, with "Restore pages?" over it when the
// profile's last exit was a crash. Answered as handled, so nothing opens.
static int CEF_CALLBACK browser_process_relaunch(cef_browser_process_handler_t *self,
                                                 cef_command_line_t *command_line,
                                                 const cef_string_t *current_directory) {
  (void)self;
  (void)current_directory;
  nd_cef_ref_release(command_line);
  fprintf(stderr, "ND_CEF_RELAUNCH_REFUSED another launch on this cache directory was turned away\n");
  return 1;
}


// MARK: - Browsers Chrome creates on its own

// Chrome style answers chrome.windows.create, chrome.tabs.create into such a
// window and chrome.runtime.openOptionsPage by building a Chrome browser window,
// which goes through no popup, open-URL or command callback. get_default_client
// is the one seam, and the client it hands out is the host's (set from Swift);
// with none set CEF builds the window unmanaged and puts it on screen.
static cef_client_t *default_client = NULL;
static cef_browser_process_handler_t *browser_process_handler = NULL;

void nd_cef_set_default_client(cef_client_t *client) {
  if (client) {
    nd_cef_ref_add(client);
  }
  if (default_client) {
    nd_cef_ref_release(default_client);
  }
  default_client = client;
}

static cef_client_t *CEF_CALLBACK process_default_client(cef_browser_process_handler_t *self) {
  (void)self;
  if (!default_client) {
    return NULL;
  }
  nd_cef_ref_add(default_client);
  return default_client;
}

static cef_browser_process_handler_t *CEF_CALLBACK app_browser_process_handler(cef_app_t *self) {
  (void)self;
  if (!browser_process_handler) {
    browser_process_handler = (cef_browser_process_handler_t *)nd_cef_ref_alloc(
        sizeof(cef_browser_process_handler_t), NULL, NULL);
    if (!browser_process_handler) {
      return NULL;
    }
    browser_process_handler->on_already_running_app_relaunch = browser_process_relaunch;
    browser_process_handler->get_default_client = process_default_client;
  }
  nd_cef_ref_add(browser_process_handler);
  return browser_process_handler;
}

// MARK: - Content blocking, renderer side

// src/adblock.zig `renderer_bootstrap`, kept byte for byte: this helper never
// links libnd. It yields the frame's content-blocking script, served by the
// host from the nd-adblock scheme, or "".
static const char adblock_bootstrap[] =
    "(function () {\n"
    "  if (location.protocol !== \"http:\" && location.protocol !== \"https:\") return \"\";\n"
    "  try {\n"
    "    var x = new XMLHttpRequest();\n"
    "    x.open(\"GET\", \"nd-adblock://frame/?u=\" + encodeURIComponent(location.href), false);\n"
    "    x.send();\n"
    "    return x.status === 200 ? x.responseText : \"\";\n"
    "  } catch (e) {\n"
    "    return \"\";\n"
    "  }\n"
    "})()";

static cef_render_process_handler_t *render_process_handler = NULL;

// CEF's eval compiles directly, so a page's CSP has no say in it. Returns the
// value when it is a non-empty string; free it with nd_cef_string_free.
static cef_string_userfree_t adblock_eval(cef_v8_context_t *context, const cef_string_t *code) {
  if (!context->eval) {
    return NULL;
  }
  cef_string_t name = {0};
  nd_cef_string_set("nd-adblock://frame/", strlen("nd-adblock://frame/"), &name);
  cef_v8_value_t *value = NULL;
  cef_v8_exception_t *exception = NULL;
  int ok = context->eval(context, code, &name, 1, &value, &exception);
  nd_cef_string_clear(&name);
  if (exception) {
    nd_cef_ref_release(exception);
  }
  cef_string_userfree_t text = NULL;
  if (ok && value && value->is_string && value->is_string(value) && value->get_string_value) {
    text = value->get_string_value(value);
    if (text && text->length == 0) {
      nd_cef_string_free(text);
      text = NULL;
    }
  }
  if (value) {
    nd_cef_ref_release(value);
  }
  return text;
}

// A frame's main world exists and none of its scripts has run: fetch and run
// its content-blocking script.
static void CEF_CALLBACK adblock_context_created(cef_render_process_handler_t *self, cef_browser_t *browser,
                                                 cef_frame_t *frame, cef_v8_context_t *context) {
  (void)self;
  if (browser) {
    nd_cef_ref_release(browser);
  }
  if (frame) {
    nd_cef_ref_release(frame);
  }
  if (!context) {
    return;
  }
  cef_string_t bootstrap = {0};
  if (nd_cef_string_set(adblock_bootstrap, strlen(adblock_bootstrap), &bootstrap)) {
    cef_string_userfree_t script = adblock_eval(context, &bootstrap);
    nd_cef_string_clear(&bootstrap);
    if (script) {
      cef_string_userfree_t rest = adblock_eval(context, script);
      if (rest) {
        nd_cef_string_free(rest);
      }
      nd_cef_string_free(script);
    }
  }
  nd_cef_ref_release(context);
}

static cef_render_process_handler_t *CEF_CALLBACK app_render_process_handler(cef_app_t *self) {
  (void)self;
  if (!render_process_handler) {
    render_process_handler = (cef_render_process_handler_t *)nd_cef_ref_alloc(
        sizeof(cef_render_process_handler_t), NULL, NULL);
    if (!render_process_handler) {
      return NULL;
    }
    render_process_handler->on_context_created = adblock_context_created;
  }
  nd_cef_ref_add(render_process_handler);
  return render_process_handler;
}

cef_app_t *nd_cef_app_create(int browser_process) {
  cef_app_t *app = (cef_app_t *)nd_cef_ref_alloc(sizeof(cef_app_t), NULL, NULL);
  if (!app) {
    return NULL;
  }
  app->on_register_custom_schemes = app_register_schemes;
  if (browser_process) {
    app->on_before_command_line_processing = app_command_line;
    app->get_browser_process_handler = app_browser_process_handler;
  } else {
    app->get_render_process_handler = app_render_process_handler;
  }
  return app;
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
