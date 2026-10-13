use std::io;
use std::path::Path;

use interprocess::local_socket::{
    GenericFilePath, ToFsName,
    tokio::{Stream, prelude::*},
};
use serde::Serialize;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::Endpoint;

/// The platform's async local socket stream (unix socket or named pipe).
pub type LocalStream = Stream;

/// Any boxed duplex byte stream; what a [`crate::rpc::Client`] connector returns.
pub type BoxStream = Box<dyn DuplexStream>;

pub trait DuplexStream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> DuplexStream for T {}

/// Longest line a peer may send. A larger one is a protocol error, not an allocation.
pub(crate) const MAX_LINE: u64 = 16 * 1024 * 1024;

pub async fn connect(endpoint: &Endpoint) -> io::Result<LocalStream> {
    Stream::connect(endpoint.name()?).await
}

/// Connects to a socket owned by another program (Discord's `discord-ipc-0`).
pub async fn connect_path(path: &Path) -> io::Result<LocalStream> {
    Stream::connect(path.to_fs_name::<GenericFilePath>()?).await
}

/// Reads one line into `buf` (cleared first). `Ok(false)` at EOF.
pub(crate) async fn read_line<R: AsyncBufRead + Unpin>(reader: &mut R, buf: &mut String) -> io::Result<bool> {
    buf.clear();
    let n = (&mut *reader).take(MAX_LINE).read_line(buf).await?;
    if n == 0 {
        return Ok(false);
    }
    if !buf.ends_with('\n') && n as u64 >= MAX_LINE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
    }
    Ok(true)
}

pub(crate) async fn write_json<W: AsyncWrite + Unpin, T: Serialize>(writer: &mut W, value: &T) -> io::Result<()> {
    let mut line = serde_json::to_vec(value).map_err(io::Error::other)?;
    line.push(b'\n');
    writer.write_all(&line).await?;
    writer.flush().await
}

/// Drives one connection: every line read goes to `on_line`, every string received on `out`
/// is written as a line. Returns on EOF, an I/O error, or when `out` closes.
pub(crate) async fn pump<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    mut out: mpsc::UnboundedReceiver<String>,
    mut on_line: impl FnMut(&str),
) -> io::Result<()> {
    let (read, mut write) = tokio::io::split(stream);
    let mut read = BufReader::new(read);
    let mut buf = String::new();
    // Two long-lived halves rather than a select per line: `read_line` is not cancel safe.
    let reader = async {
        while read_line(&mut read, &mut buf).await? {
            let line = buf.trim_end();
            if !line.is_empty() {
                on_line(line);
            }
        }
        Ok(())
    };
    let writer = async {
        while let Some(line) = out.recv().await {
            write.write_all(line.as_bytes()).await?;
            write.write_all(b"\n").await?;
            write.flush().await?;
        }
        Ok(())
    };
    tokio::select! {
        r = reader => r,
        w = writer => w,
    }
}
