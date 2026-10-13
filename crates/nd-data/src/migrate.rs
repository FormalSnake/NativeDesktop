use rusqlite::Connection;

use crate::{Error, Result};

/// One schema step. `version` is the value `PRAGMA user_version` holds once it has run.
#[derive(Clone, Copy, Debug)]
pub struct Migration {
    pub version: u32,
    pub sql: &'static str,
}

/// Applies every migration above the database's `user_version`, each in its own transaction
/// together with the version bump, so a failure leaves the previous version intact.
pub(crate) fn run(conn: &mut Connection, migrations: &[Migration]) -> Result<u32> {
    let mut last = 0;
    for m in migrations {
        if m.version <= last {
            return Err(Error::BadMigrationOrder(m.version, last));
        }
        last = m.version;
    }
    let current: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if current > last {
        return Err(Error::NewerSchema { found: current, known: last });
    }
    for m in migrations.iter().filter(|m| m.version > current) {
        let tx = conn.transaction()?;
        tx.execute_batch(m.sql)?;
        tx.pragma_update(None, "user_version", m.version)?;
        tx.commit()?;
    }
    Ok(last)
}
