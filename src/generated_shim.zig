// Named-import shim for `src/generated/widgets.zig` (M6a Task 5). The
// generated file is never hand-edited and hardcodes `@import("../protocol.zig")`,
// which only resolves inside a module whose root directory is `src/` (an
// ancestor of both `src/generated/` and `src/protocol.zig`) — so a module
// rooted directly AT `src/generated/widgets.zig` (root dir `src/generated/`)
// can never satisfy that import (it would escape the module root, which
// Zig 0.16 forbids). This shim lives at `src/` so it can relatively import
// both; `build.zig` wires it as the named module "generated" for any
// consumer (e.g. `src/gtk/style.zig`'s dedicated test root) whose own module
// root isn't `src/` and therefore can't reach `generated/widgets.zig`
// directly via `../generated/widgets.zig`. (Zig 0.16 has no
// `usingnamespace`, so this re-exports each declaration explicitly.)
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
pub const css_class_spec = g.css_class_spec;
pub const scrolledWindowInner = g.scrolledWindowInner;
