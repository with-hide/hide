//! Program copies (OSC 52) waiting to be handed on, one per pane.
//!
//! Each step between a pane's holder and a window (the device uplink, the
//! relay tap, each window's place in the hub) keeps what a pane's program
//! wrote to the clipboard until it can send it. It keeps the latest write
//! of each pane, in the order they were written, and no older one: a later
//! write of the same pane replaces it, and a write of another pane never
//! does, because the window decides which pane's copy it takes (only one it
//! shows), and a hidden pane must not push out the copy of the pane on
//! screen (with-hide/hide-private#659).

use std::collections::VecDeque;

/// The most panes with a copy waiting at one step. Copies follow an
/// operator's selection, so more than a few at once means the steps behind
/// stopped sending; the oldest then goes, and the caller logs it.
pub const MAX_WAITING_COPIES: usize = 8;

/// The latest copy of each pane not handed on yet, oldest first.
#[derive(Debug)]
pub struct WaitingCopies<T> {
    queue: VecDeque<(String, T)>,
}

impl<T> Default for WaitingCopies<T> {
    fn default() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }
}

impl<T> WaitingCopies<T> {
    /// Queues `copy` as the pane's latest, after every other pane's. At
    /// [`MAX_WAITING_COPIES`] the oldest other pane's copy is dropped and
    /// its pane returned, for the caller to log.
    pub fn push(&mut self, pane: &str, copy: T) -> Option<String> {
        self.queue.retain(|(waiting, _)| waiting != pane);
        let dropped = (self.queue.len() >= MAX_WAITING_COPIES)
            .then(|| self.queue.pop_front().map(|(pane, _)| pane))
            .flatten();
        self.queue.push_back((pane.to_owned(), copy));
        dropped
    }

    /// The oldest copy waiting, and its pane.
    pub fn pop(&mut self) -> Option<(String, T)> {
        self.queue.pop_front()
    }

    /// Drops the pane's copy: the pane is gone from this step.
    pub fn forget(&mut self, pane: &str) {
        self.queue.retain(|(waiting, _)| waiting != pane);
    }

    pub fn clear(&mut self) {
        self.queue.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(copies: &mut WaitingCopies<&'static str>) -> Vec<(String, &'static str)> {
        std::iter::from_fn(|| copies.pop()).collect()
    }

    /// A hidden pane's copy never pushes out the shown pane's; a pane's
    /// later copy replaces its own and moves behind the others.
    #[test]
    fn each_pane_keeps_its_latest_copy_in_write_order() {
        let mut copies = WaitingCopies::default();
        assert_eq!(copies.push("shown", "a"), None);
        assert_eq!(copies.push("hidden", "b"), None);
        assert_eq!(copies.push("shown", "c"), None);
        assert_eq!(
            drain(&mut copies),
            [("hidden".to_owned(), "b"), ("shown".to_owned(), "c")]
        );
        copies.push("gone", "d");
        copies.push("kept", "e");
        copies.forget("gone");
        assert_eq!(drain(&mut copies), [("kept".to_owned(), "e")]);
    }

    #[test]
    fn past_the_cap_the_oldest_pane_goes_and_is_named() {
        let mut copies = WaitingCopies::default();
        let panes = (0..=MAX_WAITING_COPIES)
            .map(|n| format!("p{n}"))
            .collect::<Vec<_>>();
        for pane in &panes[..MAX_WAITING_COPIES] {
            assert_eq!(copies.push(pane, "x"), None);
        }
        // A pane already waiting is replaced, not counted again.
        assert_eq!(copies.push("p3", "y"), None);
        assert_eq!(
            copies.push(&panes[MAX_WAITING_COPIES], "z"),
            Some("p0".to_owned())
        );
        let left = drain(&mut copies);
        assert_eq!(left.len(), MAX_WAITING_COPIES);
        assert_eq!(left.first().map(|(pane, _)| pane.as_str()), Some("p1"));
    }
}
