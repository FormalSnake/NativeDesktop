/* include/ndterm.h — the ndterm terminal-core C ABI.

   `src/core/terminal.zig` owns a PTY + a libghostty-vt terminal instance (behind
   an internal mutex + reader thread) and exposes exactly this surface. Both
   backends consume ONLY this — the GTK Zig surface calls these as Zig fns via
   `@import("../core/terminal.zig")`; the AppKit Swift surface calls them as C
   through the CNd module. Neither backend touches libghostty-vt directly.

   Lifecycle per widget: ndterm_open() at widget-create, ndterm_close() at
   widget-destroy. Rendering: ndterm_render_lock() snapshots the whole viewport
   grid under the mutex into an internal buffer, then ndterm_cell()/ndterm_cursor()
   read from that snapshot, then ndterm_render_unlock(). Input: the surface maps a
   platform key event to bytes and calls ndterm_write_input(). */
#ifndef NDTERM_H
#define NDTERM_H
#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct nd_terminal nd_terminal;

/* A flattened, render-ready cell (resolved by the core — no palette lookup left
   for the surface). `utf8` is the NUL-terminated grapheme cluster ("" = blank). */
typedef struct {
  char    utf8[16];
  uint8_t fg[3];   /* resolved rgb */
  uint8_t bg[3];   /* resolved rgb */
  uint8_t flags;   /* bit0 bold, bit1 underline, bit2 inverse, bit3 wide, bit4 wide_tail */
} nd_term_cell;

#define NDTERM_FLAG_BOLD      (1u << 0)
#define NDTERM_FLAG_UNDERLINE (1u << 1)
#define NDTERM_FLAG_INVERSE   (1u << 2)
#define NDTERM_FLAG_WIDE      (1u << 3)
#define NDTERM_FLAG_WIDE_TAIL (1u << 4)

typedef struct {
  uint16_t x, y;
  uint8_t  visible; /* 0/1 */
  uint8_t  style;   /* 0 bar, 1 block, 2 underline, 3 hollow */
} nd_term_cursor;

/* Effect callback, invoked by the core on its reader thread when a terminal
   effect fires. The surface must marshal to its UI thread before touching UI or
   emitting events. kind: 0 = title changed (`text` = new title), 1 = bell,
   2 = child exited (`code` = exit status). */
typedef void (*nd_term_effect_cb)(void *userdata, int kind, const char *text, int code);

/* --- lifecycle --- */
/* command == NULL -> $SHELL (falls back to /bin/sh); cwd == NULL -> inherit. */
nd_terminal *ndterm_open(uint16_t cols, uint16_t rows,
                         const char *command, const char *cwd,
                         nd_term_effect_cb cb, void *userdata);
void         ndterm_close(nd_terminal *t);
void         ndterm_resize(nd_terminal *t, uint16_t cols, uint16_t rows);

/* Open-time default fg/bg + 256-color palette (WP polish-1 deliverable 7).
   Pass NULL for `opts` to ndterm_open_ex/ndterm_open_virtual_ex/ndrt_open_ex
   to get exactly ndterm_open's/ndterm_open_virtual's/ndrt_open's built-in
   defaults (foreground 0xcccccc, background black, libghostty-vt's built-in
   256-color palette). Fields are independently optional. `palette_rgb`, when
   non-NULL, must point to exactly NDTERM_PALETTE_COLORS*3 bytes: packed
   r,g,b triples for palette indices 0..255 in order (indices 0-15 are the 16
   ANSI colors) — `palette_len` must equal that byte count or the palette is
   left at its default. Only read during the open call; never retained. */
#define NDTERM_PALETTE_COLORS 256
typedef struct {
  const uint8_t *palette_rgb;  /* NULL, or NDTERM_PALETTE_COLORS*3 packed r,g,b bytes */
  size_t         palette_len;  /* must equal NDTERM_PALETTE_COLORS*3 when palette_rgb != NULL */
  uint8_t        has_fg;       /* 0/1: apply fg[3] as the default foreground */
  uint8_t        fg[3];
  uint8_t        has_bg;       /* 0/1: apply bg[3] as the default background */
  uint8_t        bg[3];
} nd_term_open_opts;

/* Same as ndterm_open, plus `opts` (see nd_term_open_opts); opts == NULL
   behaves exactly like ndterm_open. */
nd_terminal *ndterm_open_ex(uint16_t cols, uint16_t rows,
                            const char *command, const char *cwd,
                            const nd_term_open_opts *opts,
                            nd_term_effect_cb cb, void *userdata);

/* --- input (already-encoded bytes -> PTY) --- */
void         ndterm_write_input(nd_terminal *t, const uint8_t *bytes, size_t len);

/* --- scroll / mouse (WP polish-1 deliverable 6) ---
   Both also work on a remote (virtual) terminal via ndrt_terminal(rt), the
   same way ndterm_render_lock/_cell/_cursor/_write_input already do — there
   is no separate ndrt_scroll_viewport/ndrt_mouse_mode. */

/* Move the client-local scrollback viewport by `delta` rows (negative = back
   into scrollback, positive = toward the live output) and let the backend
   re-render via the normal render_lock/cell/render_unlock path. Wraps
   libghostty-vt's ghostty_terminal_scroll_viewport. Safe to call on a NULL
   terminal (no-op). */
void         ndterm_scroll_viewport(nd_terminal *t, int delta);

/* Bitmask of the VT's currently-active mouse reporting modes, so a backend
   can gate SGR mouse-event encoding on whether — and in which format — the
   app enabled reporting. Returns 0 for a NULL terminal or when no mouse mode
   is set. */
#define NDTERM_MOUSE_X10        (1u << 0) /* DECSET 9: X10 mouse reporting */
#define NDTERM_MOUSE_NORMAL     (1u << 1) /* DECSET 1000: normal (click) tracking */
#define NDTERM_MOUSE_BUTTON     (1u << 2) /* DECSET 1002: button-event tracking */
#define NDTERM_MOUSE_ANY        (1u << 3) /* DECSET 1003: any-event tracking */
#define NDTERM_MOUSE_UTF8       (1u << 4) /* DECSET 1005: UTF-8 coordinate format */
#define NDTERM_MOUSE_SGR        (1u << 5) /* DECSET 1006: SGR coordinate format */
#define NDTERM_MOUSE_URXVT      (1u << 6) /* DECSET 1015: URxvt coordinate format */
#define NDTERM_MOUSE_SGR_PIXELS (1u << 7) /* DECSET 1016: SGR pixel-coordinate format */
uint32_t     ndterm_mouse_mode(nd_terminal *t);

/* --- render read ---
   ndterm_render_lock takes the mutex, runs the libghostty-vt render-state
   update, and snapshots the whole cols*rows viewport into an internal buffer;
   fills *out_cols/*out_rows. ndterm_cell/ndterm_cursor read that snapshot (valid
   only between lock and unlock). Always call ndterm_render_unlock. */
void         ndterm_render_lock(nd_terminal *t, uint16_t *out_cols, uint16_t *out_rows);
void         ndterm_cell(nd_terminal *t, uint16_t x, uint16_t y, nd_term_cell *out);
void         ndterm_cursor(nd_terminal *t, nd_term_cursor *out);
void         ndterm_default_colors(nd_terminal *t, uint8_t fg[3], uint8_t bg[3]);
void         ndterm_render_unlock(nd_terminal *t);

/* --- virtual (remote-fed) mode ---
   A virtual terminal has NO pty/fork/reader-thread (amaster == -1). Bytes are
   pushed in with ndterm_feed and pulled out through output_cb. Used by the
   remote transport (src/core/remote_terminal.zig): a daemon streams the PTY's
   output over the byte plane, this VT renders it, and keystrokes / VT query
   responses travel back over the wire instead of to a local PTY master. */

/* Output sink for VIRTUAL terminals: replaces the write(amaster) path. In
   virtual mode both keystrokes (ndterm_write_input) and VT query responses
   (the internal write_pty effect) route here instead of to a PTY master. */
typedef void (*nd_term_output_cb)(void *userdata, const uint8_t *bytes, size_t len);

/* Open a terminal with NO pty/fork/reader-thread. Bytes are pushed in via
   ndterm_feed and pulled out via output_cb. `cb` = title/bell/exit effects,
   `userdata` is passed to BOTH cb and output_cb (caller disambiguates). */
nd_terminal *ndterm_open_virtual(uint16_t cols, uint16_t rows,
                                 nd_term_effect_cb cb, nd_term_output_cb output_cb,
                                 void *userdata);
/* Same as ndterm_open_virtual, plus `opts` (see nd_term_open_opts above);
   opts == NULL behaves exactly like ndterm_open_virtual. Used by ndrt_open_ex
   (include/ndremote.h) for remote sessions. */
nd_terminal *ndterm_open_virtual_ex(uint16_t cols, uint16_t rows,
                                    const nd_term_open_opts *opts,
                                    nd_term_effect_cb cb, nd_term_output_cb output_cb,
                                    void *userdata);
/* Feed remote PTY output into the VT (virtual mode only; no-op on pty-backed).
   Thread-safe: takes the same internal mutex as ndterm_render_lock. */
void         ndterm_feed(nd_terminal *t, const uint8_t *bytes, size_t len);
/* Reset the VT to power-on defaults before the next feed — used on a snapshot/
   replay boundary (OUTPUT frame with FLAG_RESET set). */
void         ndterm_reset(nd_terminal *t);

#ifdef __cplusplus
}
#endif
#endif /* NDTERM_H */
