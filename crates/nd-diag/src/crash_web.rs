//! The web has no second process to catch a crash in: a wasm trap ends the tab's app and the
//! browser console keeps the panic message.

use std::path::Path;

pub(crate) struct CrashClient;

impl CrashClient {
    pub fn dump_into(&self, _dir: &Path) -> bool {
        false
    }
}
