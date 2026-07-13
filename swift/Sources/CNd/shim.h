// M10: zig-out/include/nd.h is a copy of this file installed by `zig build
// libnd` — nd_plugin.h is a new sibling that nd.h's `#include "nd_plugin.h"`
// needs, but build.zig only installs nd.h into zig-out/include (not
// nd_plugin.h), so the zig-out copy's nested include fails to resolve.
// Point at the source tree instead, where nd_plugin.h is a real sibling of
// nd.h, until the header-install step is fixed to copy both.
#include "../../../include/nd.h"
#include "../../../include/ndterm.h"
