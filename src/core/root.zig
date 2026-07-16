// GTK-free re-export root for `libnd`: the static-lib artifact
// (`-Dbackend=abi`) is rooted here instead of `src/gtk/main.zig`,
// which is the GTK embedder's entrypoint, not the core's.
//
// NOTE: this is a single named-module import, not a relative-path
// `@import` — Zig 0.16 forbids a module's `@import` from escaping its
// root_source_file's directory (`src/core/`), so `abi.zig`, which stays
// flat under `src/`, cannot be reached via `@import("../abi.zig")` from
// here ("import of file outside module path"). `abi.zig`
// transitively reaches every other core file via ordinary same-directory
// relative imports (abi -> {abi_backend, tree, runtime, automation,
// protocol}; tree/runtime/automation -> backend.zig -> {null_backend,
// abi_backend}) — so importing it as the one named module is sufficient to
// re-export the whole core surface. build.zig wires the name "abi" to
// `src/abi.zig`'s module.
pub const abi = @import("abi");

// Terminal core: a PTY + libghostty-vt behind the ndterm C ABI
// (include/ndterm.h), consumed by both backends. Lives in src/core/ so both
// libnd (the Swift shell) and the GTK exe reach it; its `export fn ndterm_*`
// symbols are force-retained below exactly like abi's C-ABI surface.
pub const terminal = @import("terminal.zig");

// Remote-terminal transport: a Canary byte-plane client that feeds a virtual
// ndterm (ndremote C ABI, include/ndremote.h). GTK-free / AppKit-free like
// terminal.zig; its `export fn ndrt_*` symbols are force-retained below.
pub const remote_terminal = @import("remote_terminal.zig");

// Force retention of the `export fn` C-ABI symbols in `libnd.a`: Zig's
// lazy compilation only emits code reachable from something the compiler
// keeps, and a static-lib artifact has no "keep everything exported"
// default the way an exe's `main` call graph provides — `refAllDecls`
// inside a `test {}` block doesn't help here (never runs/analyzes for a
// plain `build-lib`, only under `zig build test`). Comptime-referencing
// each export's address is enough to force analysis + emission without
// calling anything at runtime.
comptime {
    _ = &abi.nd_init;
    _ = &abi.nd_register_backend;
    _ = &abi.nd_start_runtime;
    _ = &abi.nd_start_automation;
    _ = &abi.nd_emit_event;
    _ = &abi.nd_free;
    _ = &abi.nd_set_acl;
    _ = &abi.nd_load_plugin;
    _ = &terminal.ndterm_open;
    _ = &terminal.ndterm_close;
    _ = &terminal.ndterm_resize;
    _ = &terminal.ndterm_write_input;
    _ = &terminal.ndterm_render_lock;
    _ = &terminal.ndterm_cell;
    _ = &terminal.ndterm_cursor;
    _ = &terminal.ndterm_default_colors;
    _ = &terminal.ndterm_render_unlock;
    _ = &terminal.ndterm_open_virtual;
    _ = &terminal.ndterm_feed;
    _ = &terminal.ndterm_reset;
    _ = &remote_terminal.ndrt_open;
    _ = &remote_terminal.ndrt_terminal;
    _ = &remote_terminal.ndrt_write_input;
    _ = &remote_terminal.ndrt_resize;
    _ = &remote_terminal.ndrt_close;
}

test {
    @import("std").testing.refAllDecls(@This());
}
