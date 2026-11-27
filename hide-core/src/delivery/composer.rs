//! Reading the composer of a pane whose letters, or whose Conductor's
//! mission answer, wait on a possible draft (`docs/delivery.md`, Reading the
//! composer).
//!
//! Hide routes every key to a pane but cannot see the composer, so any key
//! not followed by a submission holds the bell as a draft, and typing then
//! erasing held letters for up to the hour. Once input is quiet, the
//! doorbell asks the engine once for its visible screen with styling and judges it
//! by the agent's composer declaration (`hide_agent_adapter::composer`). Only
//! a positive "empty" releases the draft; a screen with no composer where the
//! declaration puts it (a pane too narrow, or covered) holds the letters for
//! that instead of a draft, and anything else keeps the hold as before; each
//! read is logged without the screen's text.
//!
//! The read runs on the doorbell's thread, off the runtime lock, once per
//! observed draft, state generation and pane size, again on the bell's
//! backoff only while the screen shows no composer, and within the
//! doorbell's per-pass budget of the engine calls. The same read checks each
//! agent version at moments its composer is known empty, and only a version
//! seen read empty there is released, so a screen that changed under the
//! declaration keeps holding rather than ringing over the operator's words.
//! A screen that did not read empty is a miss, logged with the step that
//! stopped it, and the version is checked again later
//! (`crate::composer_checks`).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use hide_agent_adapter::composer::{Composer, Miss, Verdict};
use serde_json::json;

use crate::engine::{Engine, EngineError, Wait};
use crate::runtime::{Runtime, delivery::Observation};

use super::Actor;

/// A visible screen is one frame of at most a few hundred rows; the engine
/// answers it from memory, so a read slower than this is a failed read.
const READ_TIMEOUT: Duration = Duration::from_secs(1);
/// How often the doorbell looks for a version to check. Each look walks the
/// observed panes under the lock, so it is not done on every pass.
pub(super) const CHECK_EVERY_MS: u64 = 5_000;

/// A composer the doorbell may read, with the facts the read is made for.
pub(crate) struct Read {
    pub observed: Observation,
    /// The adapter id.
    pub agent: &'static str,
    pub composer: Composer,
    pub version: String,
    /// The input time of the draft episode a release is read for, or the
    /// prompt hook time of the moment a check is read at.
    pub input_at_unix_ms: u64,
    /// The pane's grid as its view last reported it (`Runtime::delivery_pane_size`),
    /// which the screen is drawn at: a screen read at one size says nothing
    /// of the same screen at another.
    pub size: Option<(u16, u16)>,
}

/// The draft episode a pane's composer was read for: the agent, the input
/// and state generations, and the grid the screen was drawn at.
type Episode = (Actor, u64, Option<u64>, Option<(u16, u16)>);

/// What a release found for a pane's draft this pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Release {
    /// An engine call was made, which counts against the doorbell's
    /// per-pass budget.
    pub read: bool,
    /// The screen of this episode showed no composer where its declaration
    /// puts it: the pane is too narrow for the agent to draw one, or a menu
    /// or a dialog covers it. The letters are held for that, not for a
    /// draft, until a read finds the composer.
    pub unreadable: bool,
}

/// When a screen read with no composer on it is read again in the same
/// episode: on the doorbell's bell backoff, 5 seconds doubling to two
/// minutes, since the screen can change with nothing Hide observes (an
/// agent redrawing after a resize, a menu closing on its own).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Again {
    at: u64,
    delay: u64,
}

/// What the reader keeps between doorbell passes. Each map is keyed by a pane
/// and pruned to the panes of the pass (`retain`) or of the observations, so
/// it is bounded by the open letters and answers and the observation limit.
#[derive(Default)]
pub(crate) struct Reader {
    /// The draft episode each pane's composer was read for and, when that
    /// read could not find the composer, when it is read again.
    read: HashMap<String, (Episode, Option<Again>)>,
    /// Why a pane's draft episode is not read, logged once per change.
    skipped: HashMap<String, (u64, &'static str)>,
    /// The prompt hook moment each pane was tried at for a version check.
    attempted: HashMap<String, u64>,
    next_check_at: u64,
}

impl Reader {
    /// Forgets the panes that have no pending letter or answer any more.
    pub(crate) fn retain(&mut self, panes: impl Fn(&str) -> bool) {
        self.read.retain(|pane, _| panes(pane));
        self.skipped.retain(|pane, _| panes(pane));
    }

    /// Reads the composer of `recipient`, a letter's or an answer's, held
    /// on a possible draft, once per draft episode, and releases the draft
    /// when the screen shows the composer empty. The episode includes the
    /// pane's size, so a resized pane is read at once; a screen that showed
    /// no composer is read again on the bell's backoff within its episode,
    /// so a redraw Hide cannot observe, such as the agent's own after a
    /// resize, is read too.
    pub(crate) fn release(
        &mut self,
        owner: &Mutex<Runtime>,
        recipient: &Actor,
        now: u64,
    ) -> Release {
        let Some((episode, read)) = owner
            .lock()
            .ok()
            .and_then(|guard| guard.composer_release_due(recipient, now))
        else {
            return Release::default();
        };
        let pane = &recipient.pane_id;
        let read = match read {
            Ok(read) => read,
            Err(reason) => {
                if self.skipped.insert(pane.clone(), (episode, reason)) != Some((episode, reason)) {
                    crate::diagnostic!(json!({"component":"delivery","kind":"composer.not_read",
                        "pane_id":pane,"agent":recipient.kind,"reason":reason}));
                }
                return Release::default();
            }
        };
        let key = (
            read.observed.actor.clone(),
            read.observed.input_generation,
            read.observed.state_change_seq,
            read.size,
        );
        let last = self
            .read
            .get(pane)
            .filter(|(read, _)| *read == key)
            .map(|(_, again)| *again);
        match last {
            Some(None) => return Release::default(),
            Some(Some(again)) if now < again.at => {
                return Release {
                    read: false,
                    unreadable: true,
                };
            }
            _ => {}
        }
        let reading = screen(owner, &read);
        let unreadable = matches!(reading, Ok(Err(miss)) if miss.verdict() == Verdict::NotFound);
        let again = unreadable.then(|| {
            let delay = last
                .flatten()
                .map_or(super::doorbell::RETRY_FIRST_MS, |again| {
                    again
                        .delay
                        .saturating_mul(2)
                        .min(super::doorbell::RETRY_LIMIT_MS)
                });
            Again {
                at: now.saturating_add(delay),
                delay,
            }
        });
        self.read.insert(pane.clone(), (key, again));
        crate::diagnostic!(json!({"component":"delivery","kind":"composer.read",
            "pane_id":pane,"agent":read.agent,"version":read.version,
            "verdict":verdict_code(reading),"reason":miss_code(reading),
            "size":read.size.map(|(rows, cols)| json!({"rows":rows,"cols":cols}))}));
        if reading == Ok(Ok(()))
            && let Ok(mut guard) = owner.lock()
        {
            guard.release_empty_composer(&read.observed, now);
        }
        Release {
            read: true,
            unreadable,
        }
    }

    /// Checks one agent version's composer declaration, at most every
    /// `CHECK_EVERY_MS`, at a moment the composer is known empty: a screen
    /// read empty passes the version, and any other is a miss that waits
    /// longer before the version's next check. Returns whether an engine call
    /// was made.
    pub(crate) fn check(&mut self, owner: &Mutex<Runtime>, now: u64) -> bool {
        if now < self.next_check_at {
            return false;
        }
        self.next_check_at = now.saturating_add(CHECK_EVERY_MS);
        let Some(read) = owner.lock().ok().and_then(|guard| {
            guard.retain_composer_attempts(&mut self.attempted);
            guard.composer_check_due(&self.attempted, now)
        }) else {
            return false;
        };
        self.attempted
            .insert(read.observed.actor.pane_id.clone(), read.input_at_unix_ms);
        let reading = screen(owner, &read);
        // A failed read says nothing about the screen, so it is no miss and
        // the version waits for its next known-empty moment.
        let check = reading.ok().and_then(|judged| {
            let miss = judged.err().map(Miss::code);
            let mut guard = owner.lock().ok()?;
            Some(guard.record_composer_check(read.agent, &read.version, miss, now))
        });
        crate::diagnostic!(
            json!({"component":"delivery","kind":"composer.version_checked",
            "pane_id":read.observed.actor.pane_id,"agent":read.agent,"version":read.version,
            "verdict":verdict_code(reading),"reason":miss_code(reading),
            "misses":check.as_ref().map(|check| check.misses),
            "retry_after_ms":check.filter(|check| !check.passed)
                .map(|check| crate::composer_checks::retry_after(check.misses))})
        );
        true
    }
}

/// A screen judged by its declaration: read empty, or the step that kept it
/// from reading empty; or the code of a read that brought no screen.
type Reading = Result<Result<(), Miss>, &'static str>;

fn verdict_code(reading: Reading) -> &'static str {
    match reading {
        Ok(judged) => judged
            .map_or_else(|miss| miss.verdict(), |()| Verdict::Empty)
            .code(),
        Err(code) => code,
    }
}

fn miss_code(reading: Reading) -> Option<&'static str> {
    reading.ok()?.err().map(Miss::code)
}

/// Reads the pane's visible screen with styling and judges it. The engine
/// is looked up under the lock; the read itself runs off it.
fn screen(owner: &Mutex<Runtime>, read: &Read) -> Reading {
    let engine = owner
        .lock()
        .ok()
        .and_then(|guard| guard.delivery_engine(&read.observed.actor.device_id))
        .ok_or("pane_unavailable")?;
    judge(engine.as_ref(), read)
}

/// The refusal when a forced letter's recipient shows text in its composer.
pub(crate) const NOT_EMPTY: &str = "composer_not_empty";
/// The refusal when a forced letter's recipient's composer cannot be read
/// empty: no declaration for its kind, no composer where it is declared (a
/// menu or a dialog over it), or a failed or cut read.
pub(crate) const UNCONFIRMED: &str = "composer_unconfirmed";
/// What the doorbell holds a bell for when the screen was read and shows no
/// composer where the declaration puts it ([`clear`]): the pane is too
/// narrow for the agent to draw it, or a menu or a dialog covers it.
pub(crate) const UNREADABLE: &str = "composer_unreadable";

/// Whether a letter may be typed into `pane`, whose agent is of
/// `kind`, now: its visible screen read with
/// styling and judged by the kind's composer declaration. Any version is
/// judged, since a screen not read empty is never typed into. Runs off the
/// runtime lock. A screen with no composer is [`UNCONFIRMED`], as every
/// refusal a sender is told about names it.
pub(crate) fn forced_clear(
    engine: &dyn Engine,
    pane: &str,
    kind: &str,
) -> Result<(), &'static str> {
    clear(engine, pane, kind).map_err(|code| {
        if code == UNREADABLE {
            UNCONFIRMED
        } else {
            code
        }
    })
}

/// [`forced_clear`] for the doorbell, which tells a screen read with no
/// composer on it ([`UNREADABLE`]) from one it could not read or judge
/// ([`UNCONFIRMED`]): only the first is the operator's to clear, by
/// widening the pane or closing what covers its composer.
pub(crate) fn clear(engine: &dyn Engine, pane: &str, kind: &str) -> Result<(), &'static str> {
    let composer = hide_agent_adapter::adapter(kind)
        .and_then(|row| row.composer.supported())
        .ok_or(UNCONFIRMED)?;
    let screen = engine
        .read_screen(Wait::Small(READ_TIMEOUT), pane)
        .map_err(|_| UNCONFIRMED)?;
    if screen.truncated {
        return Err(UNCONFIRMED);
    }
    match composer.explain(&screen.text) {
        Ok(()) => Ok(()),
        Err(miss) if miss.verdict() == Verdict::Holds => Err(NOT_EMPTY),
        Err(_) => Err(UNREADABLE),
    }
}

/// What an agent's composer shows against text Hide typed into it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shows {
    Empty,
    /// The typed text and nothing else: it was never submitted.
    Typed,
    /// Anything else, which a typing or an Enter would mix with or submit.
    Other,
}

/// Reads `pane`'s composer by its agent's declaration, as `forced_clear`
/// does, and says whether it shows `text` Hide typed there. A screen that
/// cannot be read is [`UNCONFIRMED`].
pub(crate) fn shows(
    engine: &dyn Engine,
    pane: &str,
    kind: &str,
    text: &str,
) -> Result<Shows, &'static str> {
    let composer = hide_agent_adapter::adapter(kind)
        .and_then(|row| row.composer.supported())
        .ok_or(UNCONFIRMED)?;
    let screen = engine
        .read_screen(Wait::Small(READ_TIMEOUT), pane)
        .map_err(|_| UNCONFIRMED)?;
    if screen.truncated {
        return Err(UNCONFIRMED);
    }
    if composer.holds(&screen.text, text) {
        return Ok(Shows::Typed);
    }
    match composer.explain(&screen.text) {
        Ok(()) => Ok(Shows::Empty),
        Err(miss) if miss.verdict() == Verdict::Holds => Ok(Shows::Other),
        Err(_) => Err(UNCONFIRMED),
    }
}

fn judge(engine: &dyn Engine, read: &Read) -> Reading {
    let screen = engine
        .read_screen(Wait::Small(READ_TIMEOUT), &read.observed.raw_pane_id)
        .map_err(|error| match error {
            EngineError::Request(_) => "engine_parameters",
            EngineError::Answer(_) => "engine_format",
            _ => "read_failed",
        })?;
    // A cut screen may have lost the composer's rows, which no verdict
    // could tell from an empty composer.
    if screen.truncated {
        return Err("read_truncated");
    }
    Ok(read.composer.explain(&screen.text))
}
