// GTK-free re-export root for `libnd` (M6a-D4): the static-lib artifact
// (Task 5, `-Dbackend=abi`) is rooted here instead of `src/gtk/main.zig`,
// which is the GTK embedder's entrypoint, not the core's.
//
// NOTE: this is a single named-module import, not a relative-path
// `@import` — Zig 0.16 forbids a module's `@import` from escaping its
// root_source_file's directory (`src/core/`), so `abi.zig`, which stays
// flat under `src/`, cannot be reached via `@import("../abi.zig")` from
// here (verified: "import of file outside module path"). `abi.zig`
// transitively reaches every other core file via ordinary same-directory
// relative imports (abi -> {abi_backend, tree, runtime, automation,
// protocol}; tree/runtime/automation -> backend.zig -> {null_backend,
// abi_backend}) — so importing it as the one named module is sufficient to
// re-export the whole core surface. build.zig wires the name "abi" to
// `src/abi.zig`'s module.
pub const abi = @import("abi");

test {
    @import("std").testing.refAllDecls(@This());
}
