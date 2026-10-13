use std::io;
use std::path::{Path, PathBuf};

use interprocess::local_socket::{GenericFilePath, Name, ToFsName};

/// `sun_path` is 104 bytes on macOS and 108 on Linux; keep a margin for the NUL.
const MAX_UNIX_PATH: usize = 100;

/// Where a socket (or pipe) lives and the lock file that guards it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    socket: PathBuf,
    lock: PathBuf,
}

impl Endpoint {
    /// The `channel` socket of `app`: `<app data dir>/<channel>.sock` on unix (so
    /// `ND_APP_DATA_DIR` isolates tests), `\\.\pipe\nd-<app>-<channel>` on Windows. Use
    /// distinct channels for the instance lock (`"instance"`) and an app-to-daemon RPC
    /// (`"daemon"`). A data dir too deep for `sun_path` falls back to a hashed name in the
    /// temp dir.
    pub fn for_app(app: &str, channel: &str) -> Self {
        if cfg!(windows) {
            let socket = PathBuf::from(format!(r"\\.\pipe\nd-{app}-{channel}"));
            let lock = std::env::temp_dir().join(format!("nd-{app}-{channel}.lock"));
            return Self { socket, lock };
        }
        let dir = nd_core::paths::app_data_dir(app);
        let socket = dir.join(format!("{channel}.sock"));
        if socket.as_os_str().len() <= MAX_UNIX_PATH {
            return Self::at_path(socket);
        }
        let stem = format!("nd-{:016x}", fnv1a(socket.as_os_str().as_encoded_bytes()));
        Self::at_path(std::env::temp_dir().join(stem).with_extension("sock"))
    }

    /// An explicit unix socket path (or Windows pipe path). The lock file sits beside it.
    pub fn at_path(socket: impl Into<PathBuf>) -> Self {
        let socket = socket.into();
        let lock = if cfg!(windows) {
            std::env::temp_dir().join(format!("nd-{:016x}.lock", fnv1a(socket.as_os_str().as_encoded_bytes())))
        } else {
            socket.with_extension("lock")
        };
        Self { socket, lock }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    pub fn lock_path(&self) -> &Path {
        &self.lock
    }

    pub(crate) fn name(&self) -> io::Result<Name<'_>> {
        self.socket.as_path().to_fs_name::<GenericFilePath>()
    }
}

/// Stable across processes and Rust versions, unlike `DefaultHasher`.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100000001b3))
}
