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

/* --- input (already-encoded bytes -> PTY) --- */
void         ndterm_write_input(nd_terminal *t, const uint8_t *bytes, size_t len);

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
