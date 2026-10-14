//! The test harness (ARCHITECTURE.md 10.1): Playwright-shaped locators over the semantic tree,
//! the same in every tier.
//!
//! - Tier 1: [`TestHarness`] wraps an in-process `TestApp` (TestBackend, no OS); requests run
//!   through `nd_automation::handle` directly and the app settles after each.
//! - Tier 3: [`launch_app`] starts the app as its own process with `NATIVE_AUTOMATION=1` and
//!   drives it over the automation socket; [`connect_app`] attaches to one someone else started.
//!
//! Both deref to a [`Session`], which hands out [`Locator`]s:
//!
//! ```ignore
//! let app = launch_app(LaunchOptions::example("hello"))?;
//! app.get_by_test_id("add").click()?;
//! expect(&app.get_by_role(Role::Text).name("1").exact()).to_be_visible()?;
//! ```
//!
//! The API blocks instead of returning futures: tier 1 runs on the test's thread with nothing to
//! await, and tier 3 polls a socket. Waiting locators and `expect` poll until the session's
//! timeout (5 s by default; [`Session::with_timeout`]).

mod error;
mod expect;
mod in_process;
mod launch;
mod locator;
pub mod matcher;
pub mod selector;
mod session;

pub use error::Error;
pub use expect::{LocatorAssertions, expect};
pub use in_process::TestHarness;
pub use launch::{AppHandle, LaunchOptions, connect_app, example_bin, launch_app};
pub use locator::{BoundingBox, Locator, WaitState};
pub use nd_core::semantic::{Role, SemanticNode, SemanticTree, SemanticWindow};
pub use session::{Driver, Keyboard, Mouse, Session, WindowRef};

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Takes the machine-wide real-input lock (`ndshell::lock`) and, on macOS, runs the real-input
/// preflight. Hold the guard while the test moves the real cursor, activates windows or captures
/// the screen; a second run anywhere on the machine queues behind it.
pub fn real_input(purpose: &str) -> Result<ndshell::lock::MachineLock> {
    #[cfg(target_os = "macos")]
    {
        ndshell::preflight::real_input_locked(purpose).map_err(Error::Launch)
    }
    #[cfg(not(target_os = "macos"))]
    {
        ndshell::lock::acquire_bounded(purpose).map_err(|e| Error::Launch(format!("real-input lock: {e}")))
    }
}
