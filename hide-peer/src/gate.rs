//! One direction's generation, read without a lock (PRD D-62, D-66).
//!
//! A grant, a pause or a removal moves the generation of that direction.
//! A stream remembers the generation it was opened under and is over the
//! moment the gate holds another: no request, key or frame takes a lock to
//! find out, and a request that waited under an earlier grant never runs.
//! The other direction's streams and the connection are untouched.

use std::any::Any;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::Notify;

/// The generation that opens nothing: no active grant.
pub const CLOSED: u64 = 0;

#[derive(Default)]
pub struct Gate {
    generation: AtomicU64,
    moved: Notify,
}

impl Gate {
    pub fn new(generation: u64) -> Arc<Self> {
        Arc::new(Self {
            generation: AtomicU64::new(generation),
            moved: Notify::new(),
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Whether a stream may be opened under `generation` now.
    pub fn admits(&self, generation: u64) -> bool {
        generation != CLOSED && generation == self.generation()
    }

    /// Moves the gate and ends every stream opened under what it held.
    pub fn set(&self, generation: u64) {
        if self.generation.swap(generation, Ordering::AcqRel) != generation {
            self.moved.notify_waiters();
        }
    }

    /// A stream's hold on this gate at the generation it was opened under.
    pub fn pass(self: &Arc<Self>) -> Pass {
        Pass {
            generation: self.generation(),
            gate: Arc::clone(self),
            moved: None,
        }
    }
}

/// What a stream carries to know its grant still holds.
pub struct Pass {
    gate: Arc<Gate>,
    generation: u64,
    moved: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl Pass {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Another hold at the same generation, for a stream's other half.
    pub fn twin(&self) -> Self {
        Self {
            gate: Arc::clone(&self.gate),
            generation: self.generation,
            moved: None,
        }
    }

    /// One atomic read: whether the grant this stream runs under still
    /// holds.
    pub fn holds(&self) -> bool {
        self.gate.admits(self.generation)
    }

    /// `Ready` once the gate moved. Until then the task is woken when it
    /// does, so a read parked on a quiet stream ends with its grant.
    pub fn poll_moved(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        // Register before the check: a move between the two is then seen
        // by one or the other.
        let gate = &self.gate;
        let moved = self.moved.get_or_insert_with(|| {
            let gate = Arc::clone(gate);
            Box::pin(async move { gate.moved.notified().await })
        });
        if moved.as_mut().poll(cx).is_ready() {
            self.moved = None;
        }
        if self.holds() {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}

/// One half of a stream that ends with its gate: a read or a write returns
/// `ConnectionAborted` once the generation it was opened under no longer
/// holds, and a parked one is woken to find out.
pub struct Guarded<S> {
    inner: S,
    pass: Option<Pass>,
    /// What lives as long as this half does: its place in a count.
    _hold: Option<Arc<dyn Any + Send + Sync>>,
}

impl<S> Guarded<S> {
    pub fn new(inner: S, pass: Option<Pass>, hold: Option<Arc<dyn Any + Send + Sync>>) -> Self {
        Self {
            inner,
            pass,
            _hold: hold,
        }
    }

    /// Whether the grant this half runs under still holds. A half that
    /// needs no grant always does.
    pub fn holds(&self) -> bool {
        self.pass.as_ref().is_none_or(Pass::holds)
    }

    fn poll_gate(&mut self, cx: &mut Context<'_>) -> Poll<io::Error> {
        match &mut self.pass {
            Some(pass) => pass.poll_moved(cx).map(|()| {
                io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "the grant this stream ran under changed",
                )
            }),
            None => Poll::Pending,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Guarded<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Poll::Ready(error) = self.poll_gate(cx) {
            return Poll::Ready(Err(error));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Guarded<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Poll::Ready(error) = self.poll_gate(cx) {
            return Poll::Ready(Err(error));
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Poll::Ready(error) = self.poll_gate(cx) {
            return Poll::Ready(Err(error));
        }
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pass_holds_only_for_the_generation_it_was_made_under() {
        let gate = Gate::new(4);
        let pass = gate.pass();
        assert!(pass.holds() && gate.admits(4) && !gate.admits(3));
        gate.set(5);
        assert!(!pass.holds());
        assert!(gate.pass().holds());
        gate.set(CLOSED);
        assert!(
            !gate.admits(CLOSED),
            "no grant opens nothing, even at generation zero"
        );
    }

    #[tokio::test]
    async fn a_parked_stream_is_woken_when_its_gate_moves() {
        let gate = Gate::new(1);
        let mut pass = gate.pass();
        let waiting = tokio::spawn(async move {
            std::future::poll_fn(|cx| pass.poll_moved(cx)).await;
        });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        gate.set(2);
        waiting.await.unwrap();
    }
}
