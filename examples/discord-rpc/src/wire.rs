//! Discord's local IPC framing: `u32` opcode and `u32` length (both little endian), then that
//! many bytes of JSON.

use std::io;
use std::path::PathBuf;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const HANDSHAKE: u32 = 0;
pub const FRAME: u32 = 1;
pub const CLOSE: u32 = 2;
pub const PING: u32 = 3;
pub const PONG: u32 = 4;

/// Discord's own limit is 64 KiB; anything near it is garbage.
const MAX_PAYLOAD: u32 = 1 << 20;

pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, opcode: u32, payload: &serde_json::Value) -> io::Result<()> {
    let body = serde_json::to_vec(payload).map_err(io::Error::other)?;
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&opcode.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    w.write_all(&out).await?;
    w.flush().await
}

pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<(u32, serde_json::Value)> {
    let mut header = [0u8; 8];
    r.read_exact(&mut header).await?;
    let opcode = u32::from_le_bytes(header[..4].try_into().expect("4 bytes"));
    let len = u32::from_le_bytes(header[4..].try_into().expect("4 bytes"));
    if len > MAX_PAYLOAD {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "oversized discord frame"));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body).await?;
    let value = if body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&body).map_err(io::Error::other)?
    };
    Ok((opcode, value))
}

/// Every place a Discord client may have put `discord-ipc-N`, most likely first.
pub fn socket_candidates(override_dir: Option<&std::path::Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for slot in 0..10 {
        if cfg!(windows) {
            out.push(PathBuf::from(format!(r"\\.\pipe\discord-ipc-{slot}")));
            continue;
        }
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(dir) = override_dir {
            dirs.push(dir.to_path_buf());
        } else {
            for var in ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"] {
                if let Some(dir) = std::env::var_os(var).filter(|d| !d.is_empty()) {
                    let dir = PathBuf::from(dir);
                    dirs.push(dir.join("app/com.discordapp.Discord"));
                    dirs.push(dir.join("snap.discord"));
                    dirs.push(dir);
                }
            }
            dirs.push(PathBuf::from("/tmp"));
        }
        out.extend(dirs.into_iter().map(|d| d.join(format!("discord-ipc-{slot}"))));
    }
    out.dedup();
    out
}
