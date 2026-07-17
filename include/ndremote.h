/* include/ndremote.h — the ndremote remote-terminal transport C ABI.

   `src/core/remote_terminal.zig` connects to a Canary-protocol byte-plane
   daemon (docs/protocol.md §2, re-implemented standalone in Zig — NO dependency
   on the canary repo), attaches a session, and feeds its streamed PTY output
   into a VIRTUAL ndterm (include/ndterm.h, ndterm_open_virtual). Keystrokes and
   VT query responses travel back over the wire as INPUT frames.

   Both backends consume this the same way they consume ndterm: the GTK surface
   calls these as Zig fns via @import; the AppKit surface calls them as C through
   the CNd module. A single connection multiplexes every channel opened for the
   same host:port. */
#ifndef NDREMOTE_H
#define NDREMOTE_H
#include <stdint.h>
#include <stddef.h>
#include "ndterm.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Connection lifecycle, surfaced to the app via nd_rt_state_cb so a widget can
   render a banner (connecting / reconnecting / failed). */
typedef enum {
  ND_RT_CONNECTING   = 0,
  ND_RT_AUTHED       = 1,
  ND_RT_ATTACHED     = 2,
  ND_RT_RECONNECTING = 3,
  ND_RT_FAILED       = 4,
  ND_RT_CLOSED       = 5
} nd_rt_state;

/* Connection-state callback. Fires on the transport reader thread (like
   ndterm's effect cb) — the surface must marshal before touching UI.
   `detail` is an optional human-readable reason (may be NULL). */
typedef void (*nd_rt_state_cb)(void *userdata, int state, const char *detail);

typedef struct nd_remote_terminal nd_remote_terminal;

/* Open a remote terminal: dial host:port (reusing an existing connection to the
   same endpoint), AUTH with `ticket`, ATTACH `session_id` as a controller at
   cols x rows. `effect_cb` receives title/bell/exit (the ndterm effect cb, fed
   by the VT); `state_cb` receives connection-state transitions. `userdata` is
   passed to BOTH. Returns NULL if the virtual VT could not be created. */
nd_remote_terminal *ndrt_open(const char *host, uint16_t port,
                              const char *session_id, const char *ticket,
                              uint16_t cols, uint16_t rows,
                              nd_term_effect_cb effect_cb,
                              nd_rt_state_cb state_cb, void *userdata);
/* Same as ndrt_open, plus an optional open-time default fg/bg + 256-color
   palette applied to the underlying virtual ndterm (see include/ndterm.h
   `nd_term_open_opts`). `opts == NULL` behaves exactly like ndrt_open. */
nd_remote_terminal *ndrt_open_ex(const char *host, uint16_t port,
                                 const char *session_id, const char *ticket,
                                 uint16_t cols, uint16_t rows,
                                 const nd_term_open_opts *opts,
                                 nd_term_effect_cb effect_cb,
                                 nd_rt_state_cb state_cb, void *userdata);
/* The virtual ndterm render handle — the surface draws this exactly like a
   local terminal (ndterm_render_lock/_cell/_cursor/_write_input, and also
   ndterm_scroll_viewport/ndterm_mouse_mode — there is no separate
   ndrt_scroll_viewport/ndrt_mouse_mode). */
nd_terminal *ndrt_terminal(nd_remote_terminal *rt);
/* Send keystrokes (equivalent to ndterm_write_input on the render handle). */
void         ndrt_write_input(nd_remote_terminal *rt, const uint8_t *bytes, size_t len);
/* Resize the local grid and tell the server (RESIZE frame). */
void         ndrt_resize(nd_remote_terminal *rt, uint16_t cols, uint16_t rows);
/* Detach + close. Tears the shared connection down once its last channel goes. */
void         ndrt_close(nd_remote_terminal *rt);

#ifdef __cplusplus
}
#endif
#endif /* NDREMOTE_H */
