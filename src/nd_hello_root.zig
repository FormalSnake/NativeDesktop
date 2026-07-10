// Module-root shim for the `nd-hello` exe (M6a Task 5 deviation, see plan
// self-review). The plan says `src/main.zig` moves to `src/gtk/main.zig`
// verbatim, but Zig 0.16 forbids any `@import` within a module from
// escaping that module's root directory (verified: "import of file outside
// module path") — including `../`-relative imports from subdirectory files
// back up to siblings of the root. `src/generated/widgets.zig` (codegen'd,
// never hand-edited — M6a does not touch codegen) hardcodes
// `@import("../protocol.zig")`, assuming it sits one directory below
// `protocol.zig`; `src/gtk/backend.zig`/`style.zig` need the same file, plus
// `../abi.zig`/`../tree.zig` etc. For all of these relative imports to
// resolve, the exe's module root directory must be `src/` (an ancestor of
// both `src/gtk/` and `src/generated/`), not `src/gtk/` itself — so this
// file, not `src/gtk/main.zig`, is what `build.zig` roots the executable
// and its test module at.
//
// It also re-exports `generated/widgets.zig`'s public surface at its own
// top level (mirroring `src/generated_shim.zig`, used the same way for
// `src/gtk/style.zig`'s standalone test root) so `build.zig` can register
// this SAME module as its own named import "generated"
// (`exe_mod.addImport("generated", exe_mod)`, a self-alias) — `style.zig`
// and `backend.zig` both use `@import("generated")`, and Zig 0.16 forbids
// two separately-constructed modules from both reaching the same file
// (`src/protocol.zig`, via `generated/widgets.zig`'s fixed relative
// import), so the exe's own "generated" must resolve to ITSELF, not to a
// second `generated_shim`-rooted module instance.
const g = @import("generated/widgets.zig");

pub const EmitFn = g.EmitFn;
pub const initEvents = g.initEvents;
pub const create = g.create;
pub const applyProps = g.applyProps;
pub const connectEvents = g.connectEvents;
pub const appendChild = g.appendChild;
pub const insertBefore = g.insertBefore;
pub const removeChild = g.removeChild;
pub const StyleTarget = g.StyleTarget;
pub const StyleKeyDef = g.StyleKeyDef;
pub const style_keys = g.style_keys;
pub const StyleSubDef = g.StyleSubDef;
pub const style_subkeys = g.style_subkeys;

pub const main = @import("gtk/main.zig").main;

test {
    @import("std").testing.refAllDecls(@This());
}
