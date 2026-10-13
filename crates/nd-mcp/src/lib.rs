//! MCP bridge for driving a NativeDesktop app (ARCHITECTURE.md 10.7).
//!
//! Three tools: `execute({code})` runs JavaScript in QuickJS with `app` bound to the `nd-test`
//! locator API, `snapshot` returns the compact semantic tree with `[ref=eN]`, `reset` relaunches
//! the app or reconnects to the socket.

pub mod js;
pub mod ops;
pub mod server;
pub mod session;
