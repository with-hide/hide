//! Reading and writing the contract's frames on a stream.

use std::io;

use hide_peer_protocol::frame::{self, FrameError};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

fn invalid(error: FrameError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

/// Writes one JSON message ([`hide_peer_protocol::frame`]).
pub async fn write_message<W, T>(stream: &mut W, message: &T) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes = frame::encode(message).map_err(invalid)?;
    stream.write_all(&bytes).await?;
    stream.flush().await
}

/// Reads one JSON message's bytes, at most `max` of them. `None` when the
/// stream ended cleanly before a message began.
pub async fn read_message<R>(stream: &mut R, max: usize) -> io::Result<Option<Vec<u8>>>
where
    R: AsyncRead + Unpin,
{
    let mut prefix = [0u8; frame::LENGTH_BYTES];
    match stream.read_exact(&mut prefix).await {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = frame::body_length(prefix, max).map_err(invalid)?;
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).await?;
    Ok(Some(body))
}
