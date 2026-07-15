// zig-out/include/nd.h is the copy installed by `zig build libnd`, but its
// nested `#include "nd_plugin.h"` fails to resolve there: build.zig installs
// only nd.h into zig-out/include, not the nd_plugin.h sibling. Point at the
// source tree instead, where nd_plugin.h is a real sibling of nd.h, until
// the header-install step is fixed to copy both.
#include "../../../include/nd.h"
#include "../../../include/ndterm.h"
