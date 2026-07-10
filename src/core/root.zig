// GTK-free re-export root for `libnd` (M6a-D4): the static-lib artifact
// (Task 5, `-Dbackend=abi`) is rooted here instead of `src/main.zig`, which
// is the GTK embedder's entrypoint, not the core's. Every module reachable
// from here must compile without gobject imports — `automation` joins that
// guarantee once Task 4 severs its remaining GTK calls.
//
// NOTE: these are named-module imports, not relative-path `@import`s — Zig
// 0.16 forbids a module's `@import` from escaping its root_source_file's
// directory (`src/core/`), so `runtime.zig`/`tree.zig`/etc., which stay
// flat under `src/`, cannot be reached via `@import("../runtime.zig")` from
// here (verified: "import of file outside module path"). Task 5's
// `build.zig` wiring supplies each of these as a `.imports` module entry
// (the same pattern already used for `build_options`/the gobject modules)
// pointing at `src/runtime.zig` etc. — this file cannot be compiled or
// tested standalone until that wiring lands.
pub const runtime = @import("runtime");
pub const tree = @import("tree");
pub const protocol = @import("protocol");
pub const automation = @import("automation");
pub const abi = @import("abi");
pub const backend = @import("backend");

test {
    @import("std").testing.refAllDecls(@This());
}
