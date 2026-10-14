//! Wall and monotonic clocks that work in a browser. `std::time::Instant::now` and
//! `SystemTime::now` compile on `wasm32-unknown-unknown` and panic when called; on the web these
//! are `web-time`'s (`performance.now()`, `Date.now()`), elsewhere they are std's own types.

pub use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
pub use std::time::{Instant, SystemTime, UNIX_EPOCH};
#[cfg(target_arch = "wasm32")]
pub use web_time::{Instant, SystemTime, UNIX_EPOCH};
