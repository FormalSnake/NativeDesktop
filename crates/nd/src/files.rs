//! Reading and writing the files a dialog returned, on every target. On desktop these are the
//! file system's; on the web a picked file's bytes are held by the page and a path from a save
//! dialog downloads what is written to it (`nd-os-web`). App code that goes through here instead
//! of `std::fs` runs unchanged in a browser.

use std::io;
use std::path::Path;

pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    #[cfg(target_arch = "wasm32")]
    return nd_os_web::files::read(path.as_ref());
    #[cfg(not(target_arch = "wasm32"))]
    std::fs::read(path)
}

pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    #[cfg(target_arch = "wasm32")]
    return nd_os_web::files::write(path.as_ref(), bytes.as_ref());
    #[cfg(not(target_arch = "wasm32"))]
    std::fs::write(path, bytes)
}
