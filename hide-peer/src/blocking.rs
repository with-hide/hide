//! A stream as blocking `Read` and `Write` (PRD D-79).
//!
//! NodeLink already runs on blocking byte streams, on threads of its own.
//! These two types are the whole bridge from a peer stream to that code:
//! each call runs the stream's own read or write to its end on the runtime,
//! so nothing is buffered between the two and a slow reader slows its writer
//! directly. A write that a peer does not take within
//! [`limits::WRITE_WITHIN`] fails instead of holding its thread.
//!
//! Call them from a thread that is not a runtime worker.

use std::io::{self, Read, Write};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::runtime::Handle;

use crate::limits;

pub struct BlockingReader<R> {
    stream: R,
    runtime: Handle,
}

pub struct BlockingWriter<W> {
    stream: W,
    runtime: Handle,
}

/// Wraps the two halves of a stream for a blocking caller.
pub fn blocking<W, R>(send: W, recv: R, runtime: Handle) -> (BlockingWriter<W>, BlockingReader<R>) {
    (
        BlockingWriter {
            stream: send,
            runtime: runtime.clone(),
        },
        BlockingReader {
            stream: recv,
            runtime,
        },
    )
}

impl<R: AsyncRead + Unpin> Read for BlockingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.runtime.block_on(self.stream.read(buf))
    }
}

fn stalled() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "the peer took no bytes in time")
}

impl<W: AsyncWrite + Unpin> Write for BlockingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let stream = &mut self.stream;
        self.runtime.block_on(async {
            tokio::time::timeout(limits::WRITE_WITHIN, stream.write(buf))
                .await
                .map_err(|_| stalled())?
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        let stream = &mut self.stream;
        self.runtime.block_on(async {
            tokio::time::timeout(limits::WRITE_WITHIN, stream.flush())
                .await
                .map_err(|_| stalled())?
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_blocking_thread_reads_what_the_other_end_wrote_and_sees_its_end() {
        let (near, far) = tokio::io::duplex(8);
        let (near_recv, near_send) = tokio::io::split(near);
        let (mut writer, mut reader) = blocking(near_send, near_recv, Handle::current());

        // Longer than the pipe holds, so the writer waits on the reader.
        let sent: Vec<u8> = (0..=255).collect();
        let echo = tokio::spawn(async move {
            let (mut far_recv, mut far_send) = tokio::io::split(far);
            let mut got = vec![0u8; 256];
            far_recv.read_exact(&mut got).await.unwrap();
            far_send.write_all(&got).await.unwrap();
            // Dropping both halves ends the stream.
        });
        let expected = sent.clone();
        // A thread that is not a runtime worker, as NodeLink's are.
        let back = tokio::task::spawn_blocking(move || {
            writer.write_all(&sent).unwrap();
            writer.flush().unwrap();
            drop(writer);
            let mut back = Vec::new();
            reader.read_to_end(&mut back).unwrap();
            back
        })
        .await
        .unwrap();
        assert_eq!(back, expected);
        echo.await.unwrap();
    }
}
