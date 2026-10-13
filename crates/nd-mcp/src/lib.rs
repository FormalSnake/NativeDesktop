//! MCP bridge for driving a NativeDesktop app (ARCHITECTURE.md 10.7).
//!
//! Three tools: `execute({code})` runs JavaScript in QuickJS with `app` bound, `snapshot` returns
//! the compact tree with `[ref=eN]`, `reset` relaunches or reconnects. The app sits behind the
//! [`driver::Driver`] trait; see that module for what is stubbed until `nd-automation` lands.

pub mod driver;
pub mod js;
pub mod render;
pub mod server;
pub mod session;
#[cfg(feature = "stub")]
pub mod stub;
