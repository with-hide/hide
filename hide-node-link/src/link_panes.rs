//! A machine's pane holders as the core's own engine asks them, through
//! that machine's node link (PRD engine-bundle-3-core-owns D-22).
//!
//! [`LinkPanes`] is the one [`PaneHolders`] the engine of any machine holds:
//! the core's own node answers it in the daemon's process and a device's
//! node at the far end of its link, with the same requests
//! ([`Call::Pane`], [`Call::PaneWatch`]), so whoever asks a machine's panes
//! picks the machine by its node and nothing else. Every request blocks
//! until the node answers or its bound passes, so none is made under the
//! runtime lock.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::holders::{
    HolderError, HolderProcesses, HolderProof, HolderRecovery, HolderStart, PaneCall, PaneEnded,
    PaneFactsSink, PaneHolders, PaneInput, PaneText, PaneWatchReport, ScreenRules, StyledRead,
};
use crate::link::{LinkAnswer, LinkError, NodeLink};
use crate::protocol::{Call, ShellLaunch};

/// How long a start may take on the node: its holder says it is ready
/// within ten seconds, and a start that finds an earlier one ending waits
/// for that end.
const START_WAIT: Duration = Duration::from_secs(40);
/// How long an end may take: a holder's close is bounded at 20 s, and a
/// tree that ignores it is ended by its pids after that.
const END_WAIT: Duration = Duration::from_secs(50);
/// How long an ask, an input or new rules may take.
const ASK_WAIT: Duration = Duration::from_secs(15);
/// What a letter may take beyond its gap.
const LETTER_WAIT: Duration = Duration::from_secs(15);
/// How long one pane watch call is held open before it is opened again, so
/// no deadline overflows; each new watch is told every pane's facts.
const WATCH_SPAN: Duration = Duration::from_secs(24 * 60 * 60);
/// How long a watch that could not be opened waits before it tries again.
const WATCH_RETRY: Duration = Duration::from_secs(1);

/// A machine's holders, reached by its node `link`.
pub struct LinkPanes {
    inner: Arc<Inner>,
}

/// Where the holders' own records go: the diagnostic log of the process
/// that holds them.
pub type Diagnose = Arc<dyn Fn(serde_json::Value) + Send + Sync>;

struct Inner {
    link: Arc<dyn NodeLink>,
    diagnose: Diagnose,
    sinks: Mutex<Sinks>,
    /// Raised when the engine lets go of these holders: the watch answers
    /// its next report with false and ends.
    stopped: AtomicBool,
}

#[derive(Default)]
struct Sinks {
    facts: Option<PaneFactsSink>,
    ends: Option<PaneEnded>,
    /// Whether the watch thread was started.
    watching: bool,
}

impl LinkPanes {
    /// The holders of the machine whose node `link` reaches; what goes
    /// wrong with following them goes to `diagnose`.
    pub fn new(link: Arc<dyn NodeLink>, diagnose: Diagnose) -> Self {
        Self {
            inner: Arc::new(Inner {
                link,
                diagnose,
                sinks: Mutex::default(),
                stopped: AtomicBool::new(false),
            }),
        }
    }

    fn ask(&self, call: PaneCall, wait: Duration) -> Result<serde_json::Value, HolderError> {
        let answer = self
            .inner
            .link
            .call(Call::Pane { call }, wait)
            .map_err(link_failure)?;
        let answer: crate::holders::PaneAnswer = decode(answer)?;
        answer
    }

    fn done(&self, call: PaneCall, wait: Duration) -> Result<(), HolderError> {
        self.ask(call, wait).map(drop)
    }

    fn answer<T: serde::de::DeserializeOwned>(
        &self,
        call: PaneCall,
        wait: Duration,
    ) -> Result<T, HolderError> {
        let value = self.ask(call, wait)?;
        serde_json::from_value(value).map_err(|error| HolderError::Unreachable {
            message: format!("the node answered a pane request in an unexpected shape: {error}"),
        })
    }

    /// Starts the one thread that follows the node's panes, the first time
    /// the engine asks to be told of them.
    fn watch(&self, set: impl FnOnce(&mut Sinks)) {
        let start = {
            let mut sinks = self.inner.sinks.lock().unwrap_or_else(PoisonError::into_inner);
            set(&mut sinks);
            !std::mem::replace(&mut sinks.watching, true)
        };
        if !start {
            return;
        }
        let inner = Arc::clone(&self.inner);
        let spawned = std::thread::Builder::new()
            .name("hide-pane-watch".to_owned())
            .spawn(move || inner.follow());
        if let Err(error) = spawned {
            // Without the watch no pane's facts or end reach the engine; the
            // panes still run and answer every other request.
            (self.inner.diagnose)(serde_json::json!({
                "component": "pane_watch",
                "kind": "pane_watch.unstarted",
                "error": error.to_string(),
            }));
        }
    }
}

impl Drop for LinkPanes {
    fn drop(&mut self) {
        self.inner.stopped.store(true, Ordering::Release);
    }
}

impl Inner {
    /// Holds a pane watch open on the node until the engine lets go or the
    /// link ends, opening it again when it ends any other way.
    fn follow(&self) {
        while !self.stopped.load(Ordering::Acquire) && self.link.closed_reason().is_none() {
            let mut undecoded = None;
            let ended = self.link.call_with_progress(
                Call::PaneWatch,
                WATCH_SPAN,
                &mut |report| {
                    if self.stopped.load(Ordering::Acquire) {
                        return false;
                    }
                    match serde_json::from_value::<PaneWatchReport>(report) {
                        Ok(report) => {
                            self.tell(report);
                            true
                        }
                        Err(error) => {
                            undecoded = Some(error.to_string());
                            false
                        }
                    }
                },
            );
            if self.stopped.load(Ordering::Acquire) {
                return;
            }
            let reason = match (undecoded, ended) {
                (Some(error), _) => format!("a report in an unexpected shape: {error}"),
                (None, Ok(_)) => "the node ended the watch".to_owned(),
                (None, Err(error)) => error.to_string(),
            };
            (self.diagnose)(serde_json::json!({
                "component": "pane_watch",
                "kind": "pane_watch.ended",
                "reason": reason,
            }));
            std::thread::sleep(WATCH_RETRY);
        }
    }

    fn tell(&self, report: PaneWatchReport) {
        let (facts, ends) = {
            let sinks = self.sinks.lock().unwrap_or_else(PoisonError::into_inner);
            (sinks.facts.clone(), sinks.ends.clone())
        };
        match report {
            PaneWatchReport::Facts { pane, facts: told } => {
                if let Some(facts) = facts {
                    facts(&pane, &told);
                }
            }
            PaneWatchReport::Ended { pane } => {
                if let Some(ends) = ends {
                    ends(&pane);
                }
            }
            PaneWatchReport::Quiet => {}
        }
    }
}

impl PaneHolders for LinkPanes {
    fn start(&self, start: &HolderStart) -> Result<(), HolderError> {
        self.done(
            PaneCall::Start {
                start: start.clone(),
            },
            START_WAIT,
        )
    }

    fn shell_launch(&self) -> Option<ShellLaunch> {
        self.answer::<Option<ShellLaunch>>(PaneCall::ShellLaunch, ASK_WAIT)
            .ok()
            .flatten()
    }

    fn terminate(&self, pane: &str) -> Result<(), HolderError> {
        self.done(
            PaneCall::Terminate {
                pane: pane.to_owned(),
            },
            END_WAIT,
        )
    }

    fn read(&self, pane: &str, lines: u16) -> Result<(String, bool), HolderError> {
        let text: PaneText = self.answer(
            PaneCall::Read {
                pane: pane.to_owned(),
                lines,
            },
            ASK_WAIT,
        )?;
        Ok((text.text, text.truncated))
    }

    fn read_styled(&self, pane: &str, read: StyledRead) -> Result<(String, bool), HolderError> {
        let text: PaneText = self.answer(
            PaneCall::ReadStyled {
                pane: pane.to_owned(),
                read,
            },
            ASK_WAIT,
        )?;
        Ok((text.text, text.truncated))
    }

    fn processes(&self, pane: &str) -> Result<HolderProcesses, HolderError> {
        self.answer(
            PaneCall::Processes {
                pane: pane.to_owned(),
            },
            ASK_WAIT,
        )
    }

    fn proof(&self, pane: &str) -> Result<HolderProof, HolderError> {
        self.answer(
            PaneCall::Proof {
                pane: pane.to_owned(),
            },
            ASK_WAIT,
        )
    }

    fn input(&self, pane: &str, input: PaneInput) -> Result<(), HolderError> {
        self.done(
            PaneCall::Input {
                pane: pane.to_owned(),
                input,
            },
            ASK_WAIT,
        )
    }

    fn letter(
        &self,
        pane: &str,
        body: PaneInput,
        submit: PaneInput,
        gap: Duration,
    ) -> Result<(), HolderError> {
        self.done(
            PaneCall::Letter {
                pane: pane.to_owned(),
                body,
                submit,
                gap_ms: u64::try_from(gap.as_millis()).unwrap_or(u64::MAX),
            },
            gap.saturating_add(LETTER_WAIT),
        )
    }

    fn recovered(&self, timeout: Duration) -> Result<HolderRecovery, HolderError> {
        self.answer(
            PaneCall::Recovered {
                timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            },
            timeout.saturating_add(ASK_WAIT),
        )
    }

    fn watch_ends(&self, ended: PaneEnded) {
        self.watch(|sinks| sinks.ends = Some(ended));
    }

    fn watch_facts(&self, sink: PaneFactsSink) {
        self.watch(|sinks| sinks.facts = Some(sink));
    }

    fn set_rules(&self, pane: &str, rules: Option<ScreenRules>) -> Result<(), HolderError> {
        self.done(
            PaneCall::SetRules {
                pane: pane.to_owned(),
                rules,
            },
            ASK_WAIT,
        )
    }
}

/// A link that did not carry a pane request, in the holders' words: the
/// node refused it under its own code, or could not be reached.
fn link_failure(error: LinkError) -> HolderError {
    match error {
        LinkError::Refused(error) => HolderError::Refused {
            code: error.code.as_str().to_owned(),
            message: error.message,
        },
        error => HolderError::Unreachable {
            message: error.to_string(),
        },
    }
}

fn decode<T: serde::de::DeserializeOwned>(answer: LinkAnswer) -> Result<T, HolderError> {
    let decoded = match answer {
        LinkAnswer::Parsed(value) => serde_json::from_value(value),
        LinkAnswer::Raw(raw) => serde_json::from_str(raw.get()),
    };
    decoded.map_err(|error| HolderError::Unreachable {
        message: format!("the node answered a pane request in an unexpected shape: {error}"),
    })
}
