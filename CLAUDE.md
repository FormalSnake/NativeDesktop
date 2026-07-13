# CLAUDE.md

Project-level guidance for NativeDesktop — a cross-platform native UI toolkit
where one React/TSX codebase renders to real native widgets (libadwaita/GTK on
Linux, AppKit on macOS).

## Fix UI at the framework level, not the app level

When asked to fix how the UI looks or feels on a platform — spacing, native
chrome, sidebar/selection styling, control metrics, anything that reads as
"not native" — the fix belongs in the **platform backend** (`swift/Sources/
NDShell`, `swift/Sources/NDGen`, or the Zig/GTK side), so the **same, unchanged
app tree** renders natively. Do NOT reach for per-app pixel-tuning in an example
or app (hand-set `padding`, one-off `cssClasses`, magic offsets).

The toolkit's whole promise is **one codebase → native on every platform**.
App-level tuning breaks that promise: it fixes one screen, doesn't generalize,
and drifts from native as the OS evolves. A structural class like
`navigation-sidebar` should carry real native semantics on each backend, not be
a no-op that apps then paper over with manual styling.

Platforms may legitimately look *different from each other* — accept that, as
long as each one looks native **for its own OS**. Match the platform, not the
other platform.

Fall back to changing app/example code only when the tree genuinely cannot
express the intent — and when you do, say so explicitly rather than editing
silently.
