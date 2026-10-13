//! Local IPC for nd apps (ARCHITECTURE.md 2): JSON lines over a unix socket or Windows named
//! pipe. Three layers share one [`Endpoint`]:
//!
//! - [`instance`]: single-instance lock; a second launch forwards its args and URLs to the
//!   running one and exits.
//! - [`rpc`]: JSON-RPC 2.0 with typed [`rpc::Method`] contracts, a [`rpc::Server`] and a
//!   [`rpc::Client`] that reconnects on a [`ladder::Ladder`].
//! - [`connect`]: a raw local-socket stream, for protocols that are not JSON lines (Discord).

mod endpoint;
mod framing;
pub mod instance;
pub mod ladder;
pub mod rpc;

pub use endpoint::Endpoint;
pub use framing::{BoxStream, LocalStream, connect, connect_path};
