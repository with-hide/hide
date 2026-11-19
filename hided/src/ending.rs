//! An end that stays announced: a role's end, and the process's stop
//! request. Each is level-triggered, so a task that looks only after it was
//! announced, such as a supervisor busy starting a role when `hide stop`
//! arrives, still sees it; a `Notify` would have dropped it.

use std::sync::Arc;

use tokio::sync::watch;

#[derive(Clone, Debug)]
pub struct Ending(Arc<watch::Sender<bool>>);

impl Default for Ending {
    fn default() -> Self {
        Self(Arc::new(watch::Sender::new(false)))
    }
}

impl Ending {
    pub fn new() -> Self {
        Self::default()
    }

    /// Announces the end; it is never taken back.
    pub fn end(&self) {
        self.0.send_replace(true);
    }

    pub fn has_ended(&self) -> bool {
        *self.0.borrow()
    }

    /// The end as a channel, for a part outside this crate (the opener).
    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.0.subscribe()
    }

    /// Returns once the end is announced, at once when it already was.
    pub async fn ended(&self) {
        let mut ended = self.0.subscribe();
        // The sender is this value's own, so the wait cannot lose it.
        let _ = ended.wait_for(|ended| *ended).await;
    }

    /// The process's stop request: SIGTERM, which `hide stop` sends, or
    /// SIGINT on Unix; Ctrl+C, Ctrl+Break or the console closing on
    /// Windows. The handlers are installed before this returns, so a signal
    /// any time after it is kept. Where a handler cannot be installed no
    /// stop is ever announced, and the process stops only on its own
    /// shutdown. One per process.
    pub fn on_stop_signal() -> Self {
        let ending = Self::new();
        let signalled = stop_signal();
        let announced = ending.clone();
        tokio::spawn(async move {
            signalled.await;
            announced.end();
        });
        ending
    }
}

#[cfg(unix)]
fn stop_signal() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut terminate), Ok(mut interrupt)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) else {
        return Box::pin(std::future::pending());
    };
    Box::pin(async move {
        tokio::select! {
            _ = terminate.recv() => {}
            _ = interrupt.recv() => {}
        }
    })
}

#[cfg(windows)]
fn stop_signal() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    use tokio::signal::windows::{ctrl_break, ctrl_c, ctrl_close};
    let (Ok(mut interrupt), Ok(mut break_key), Ok(mut close)) =
        (ctrl_c(), ctrl_break(), ctrl_close())
    else {
        return Box::pin(std::future::pending());
    };
    Box::pin(async move {
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = break_key.recv() => {}
            _ = close.recv() => {}
        }
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// A stop signalled before anything waits on it is still announced to
    /// the first wait that looks, and to every later one.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stop_signalled_before_anything_waits_is_kept() {
        let stop = Ending::on_stop_signal();
        hide_platform::process::terminate(std::process::id()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), stop.ended())
            .await
            .expect("the stop was lost");
        tokio::time::timeout(Duration::from_millis(100), stop.ended())
            .await
            .expect("a later wait missed the stop");
        assert!(stop.has_ended());
    }
}
