//! The in-app automation server (ARCHITECTURE.md 10.1): JSON-RPC over a per-app socket, served
//! from the semantic tree, enabled with `NATIVE_AUTOMATION=1`.
//!
//! The RPC vocabulary is the old repo's (`schema/rpc.json`), kept so drive scripts and agents
//! carry over: `getTree`, `click`, `doubleClick`, `rightClick`, `hover`, `focus`, `setValue`,
//! `type`, `scroll`, `scrollIntoView`, `keys`, `pointer`, `drag`, `waitFor`, `screenshot`,
//! `windows`, `resolve`. Every targeting method takes exactly one of `ref` / `testId`.
//!
//! [`handle`] runs one request against an [`App`] on its own thread; tier 1 tests call it
//! in-process, the socket [`server`] calls it on the main thread through an [`Executor`].
//! `waitFor` polls [`wait_check`] host-side every 50 ms instead of a client round-trip per tick.

#[cfg(feature = "server")]
mod handler;
pub mod protocol;
#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "server")]
pub use handler::{WaitParams, handle, wait_check};
pub use protocol::{RpcError, RpcErrorCode};
#[cfg(feature = "server")]
pub use server::{Executor, Job, start, start_from_env};
