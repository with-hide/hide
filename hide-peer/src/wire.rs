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
/// stream ended cleanly before a message began. Not cancel-safe: a call
/// dropped part way loses the bytes it read, so a `select!` that loops reads
/// through a [`MessageReader`].
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

/// Reads JSON messages off one receiving half, for a wait that may be
/// dropped and taken up again: a branch of a `select!` that loops. What was
/// read of the message it is on is kept here, never in the future that
/// waits, and it never reads past that message, so a dropped wait loses
/// nothing and the next one goes on where it stopped.
pub struct MessageReader<R> {
    stream: R,
    max: usize,
    prefix: [u8; frame::LENGTH_BYTES],
    /// How much of the prefix, then of the body, is read.
    read: usize,
    /// The message's body, from when its prefix is whole.
    body: Option<Vec<u8>>,
}

impl<R> MessageReader<R>
where
    R: AsyncRead + Unpin,
{
    /// Reads `stream`'s messages, each at most `max` bytes.
    pub fn new(stream: R, max: usize) -> Self {
        Self {
            stream,
            max,
            prefix: [0u8; frame::LENGTH_BYTES],
            read: 0,
            body: None,
        }
    }

    /// The next message's bytes. `None` when the stream ended between two
    /// messages; an end inside one is an error. Cancel-safe.
    pub async fn next(&mut self) -> io::Result<Option<Vec<u8>>> {
        loop {
            let (into, on_prefix) = match self.body.as_mut() {
                Some(body) => (&mut body[self.read..], false),
                None => (&mut self.prefix[self.read..], true),
            };
            if into.is_empty() {
                self.read = 0;
                if !on_prefix {
                    return Ok(self.body.take());
                }
                // The length is judged before a byte of the body is asked for.
                let length = frame::body_length(self.prefix, self.max).map_err(invalid)?;
                self.body = Some(vec![0u8; length]);
                continue;
            }
            // One `read` is cancel-safe: dropped before it answers, it took
            // no byte; once it answered, the count below is kept at once.
            let read = self.stream.read(into).await?;
            if read == 0 {
                return if on_prefix && self.read == 0 {
                    Ok(None)
                } else {
                    Err(io::ErrorKind::UnexpectedEof.into())
                };
            }
            self.read += read;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::AsyncWriteExt;

    use super::*;

    const MAX: usize = 1024;

    fn framed(text: &str) -> Vec<u8> {
        frame::encode(&text).unwrap()
    }

    /// A wait dropped while a message is part way in loses nothing: the
    /// reader keeps what it read, and the next wait ends the message. Every
    /// split of two messages is tried, the wait dropped after each part.
    #[tokio::test(start_paused = true)]
    async fn a_reader_whose_wait_is_dropped_part_way_through_a_message_still_reads_it_whole() {
        let bytes = [framed("the first message"), framed("and a second")].concat();
        for split in 1..bytes.len() {
            let (mut near, far) = tokio::io::duplex(64);
            let mut reader = MessageReader::new(far, MAX);
            let mut heard = Vec::new();
            for part in [&bytes[..split], &bytes[split..]] {
                near.write_all(part).await.unwrap();
                // The wait a `select!` drops when another branch wins.
                while let Ok(message) =
                    tokio::time::timeout(Duration::from_millis(250), reader.next()).await
                {
                    heard.push(message.unwrap().unwrap());
                }
            }
            assert_eq!(
                heard,
                [
                    serde_json::to_vec("the first message").unwrap(),
                    serde_json::to_vec("and a second").unwrap()
                ],
                "split at byte {split}"
            );
        }
    }

    #[tokio::test]
    async fn a_reader_tells_an_end_between_messages_from_one_inside_a_message() {
        let bytes = framed("whole");
        // Between two messages: the stream's clean end.
        let (mut near, far) = tokio::io::duplex(64);
        near.write_all(&bytes).await.unwrap();
        drop(near);
        let mut reader = MessageReader::new(far, MAX);
        assert_eq!(
            reader.next().await.unwrap(),
            Some(serde_json::to_vec("whole").unwrap())
        );
        assert_eq!(reader.next().await.unwrap(), None);
        // Inside the prefix, and inside the body.
        for cut in [1, frame::LENGTH_BYTES + 2] {
            let (mut near, far) = tokio::io::duplex(64);
            near.write_all(&bytes[..cut]).await.unwrap();
            drop(near);
            let ended = MessageReader::new(far, MAX).next().await;
            assert_eq!(
                ended.unwrap_err().kind(),
                io::ErrorKind::UnexpectedEof,
                "cut at byte {cut}"
            );
        }
    }

    /// A message longer than the reader takes is refused when its prefix is
    /// whole, before a byte of its body is asked for.
    #[tokio::test]
    async fn a_reader_refuses_a_message_past_its_limit_at_the_prefix() {
        let bytes = framed(&"x".repeat(MAX));
        let (mut near, far) = tokio::io::duplex(64);
        near.write_all(&bytes[..frame::LENGTH_BYTES]).await.unwrap();
        let refused = MessageReader::new(far, MAX).next().await;
        assert_eq!(refused.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }
}
