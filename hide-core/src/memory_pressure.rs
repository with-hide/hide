//! Each machine's memory pressure, as its own node answers it
//! (`Call::MemoryPressure`), for agent sleep's pressure rule.
//!
//! The read never runs under `Mutex<Runtime>`: a machine's session-sync
//! coordinator drives a [`PressureReader`], whose worker asks the node and
//! hands the answer in, and the minute decision acts on the last answer it
//! holds. A device answers with its own machine's pressure, so an agent there
//! sleeps by the memory it uses.

use std::sync::Arc;
use std::time::Duration;

use hide_node_link::memory::{MemoryPressure, PressureAnswer};
use hide_node_link::protocol::Call;

use crate::agent_sleep::DECISION_INTERVAL_MS;
use crate::node_access::{NodeLink, call_as};

/// Read four times for each decision, so the minute acts on a level at most
/// this old: once pressure ends, the next decision sees it. The read is one
/// kernel value or one small file on the machine.
const READ_INTERVAL: Duration = Duration::from_secs(15);
/// The node reads one kernel value or one small file; a machine too pressed
/// or too far to answer in this is unread for the minute.
const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// An answer older than this, from a reader that stalled, is no answer: four
/// reads' time, so a stalled reader sleeps one agent at most.
const FRESH_FOR_MS: u64 = DECISION_INTERVAL_MS;

/// One machine's last answer, and when it came.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PressureReading {
    pub(crate) answer: Result<PressureAnswer, String>,
    pub(crate) read_at_unix_ms: u64,
}

impl PressureReading {
    /// The answer the minute decision sleeps an agent for: a fresh one of
    /// warning or critical. A failed, stale or normal reading sleeps nothing.
    pub(crate) fn pressing(&self, now_unix_ms: u64) -> Option<PressureAnswer> {
        if now_unix_ms.saturating_sub(self.read_at_unix_ms) > FRESH_FOR_MS {
            return None;
        }
        match self.answer {
            Ok(
                answer @ PressureAnswer {
                    level: MemoryPressure::Warning | MemoryPressure::Critical,
                    ..
                },
            ) => Some(answer),
            Ok(_) | Err(_) => None,
        }
    }
}

/// Reads one machine's pressure on a worker, every [`READ_INTERVAL`].
pub(crate) struct PressureReader {
    inner: crate::reader::BackgroundRead<(), PressureReading>,
}

impl PressureReader {
    /// Reads the pressure of `node`'s machine.
    pub(crate) fn new(node: Arc<dyn NodeLink>) -> Self {
        Self {
            inner: crate::reader::BackgroundRead::new(
                READ_INTERVAL,
                Duration::ZERO,
                move |_: &()| read_now(node.as_ref()),
            ),
        }
    }

    /// An answer on the wake its worker finishes; `None` otherwise.
    pub(crate) fn read_if_due(&mut self) -> Option<PressureReading> {
        self.inner.poll(())
    }
}

/// The pressure of `node`'s machine now. Waits on the node, so only a
/// worker may call it; a node that cannot answer, one too old to know the
/// call included, is a failed read.
pub(crate) fn read_now(node: &dyn NodeLink) -> PressureReading {
    let answer = call_as::<PressureAnswer>(node, Call::MemoryPressure, READ_TIMEOUT)
        .map_err(|error| error.to_string());
    PressureReading {
        answer,
        read_at_unix_ms: crate::runtime::unix_milliseconds(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_node_link::memory::PressureSource;

    fn reading(level: Result<MemoryPressure, String>) -> PressureReading {
        PressureReading {
            answer: level.map(|level| PressureAnswer {
                level,
                source: PressureSource::System,
            }),
            read_at_unix_ms: 1_000_000,
        }
    }

    fn level(reading: &PressureReading, now: u64) -> Option<MemoryPressure> {
        reading.pressing(now).map(|answer| answer.level)
    }

    /// Warning and critical press while fresh; normal, an error and an
    /// answer from a reader that stalled do not.
    #[test]
    fn only_a_fresh_warning_or_critical_answer_presses() {
        let now = 1_000_000 + DECISION_INTERVAL_MS;
        assert_eq!(
            level(&reading(Ok(MemoryPressure::Warning)), now),
            Some(MemoryPressure::Warning)
        );
        assert_eq!(
            level(&reading(Ok(MemoryPressure::Critical)), now),
            Some(MemoryPressure::Critical)
        );
        assert_eq!(level(&reading(Ok(MemoryPressure::Normal)), now), None);
        assert_eq!(level(&reading(Err("no psi".into())), now), None);
        let stale = now + 1;
        assert_eq!(level(&reading(Ok(MemoryPressure::Critical)), stale), None);
    }
}
