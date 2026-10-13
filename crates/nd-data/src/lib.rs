//! SQLite for nd apps (ARCHITECTURE.md 2): one connection on its own thread, an async API in
//! front of it. Nothing here touches the UI thread or names an executor; the futures are plain
//! oneshot receivers, so they run under GPUI's `cx.spawn`, tokio or `block_on` alike.
//!
//! Ports `@nativedesktop/data`: `query` / `mutate` / `transaction` / `close`, plus the
//! [`SqliteExecutor`] contract an ORM adapter targets. The crate depends on no ORM.

mod database;
mod migrate;
mod value;

pub use database::{Database, OpenOptions, SqliteExecutor};
pub use migrate::Migration;
pub use rusqlite;
pub use value::{Row, RunResult, Statement, StepResult, Value};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("database i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("database is closed")]
    Closed,
    #[error("database thread panicked: {0}")]
    Panicked(String),
    #[error("database is at schema version {found}, newer than the {known} this build knows")]
    NewerSchema { found: u32, known: u32 },
    #[error("migration versions must be positive and strictly increasing, got {0} after {1}")]
    BadMigrationOrder(u32, u32),
    #[error("row did not match the target type: {0}")]
    Decode(#[from] serde_json::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
