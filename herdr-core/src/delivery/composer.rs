//! Reading the composer of a pane whose letters wait on a possible draft
//! (`docs/delivery.md`, Reading the composer).
//!
//! Hide routes every key to a pane but cannot see the composer, so any key
//! not followed by a submission holds the bell as a draft, and typing then
//! erasing held letters for up to the hour. When such a pane rests, the
//! doorbell asks Herdr once for its visible screen with styling and judges it
//! by the agent's composer declaration (`hide_agent_adapter::composer`). Only
//! a positive "empty" releases the draft; anything else keeps the hold as
//! before and is logged without the screen's text.
//!
//! The read runs on the doorbell's thread, off the runtime lock, at most
//! once per draft episode (the input time that made the draft) and within
//! the doorbell's per-pass budget of Herdr calls. The same read checks each
//! agent version once at a moment its composer is known empty, so a screen
//! that changed under the declaration turns the release off for that
//! version rather than ringing over the operator's words.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use hide_agent_adapter::composer::{Composer, Verdict};
use hide_herdr_client::{ApiConnector, request_small_response};
use serde_json::json;

use crate::runtime::Runtime;
use crate::wire;

use super::Actor;

/// A visible screen is one frame of at most a few hundred rows; Herdr
/// answers it from memory, so a read slower than this is a failed read.
const READ_TIMEOUT: Duration = Duration::from_secs(1);
/// How often the doorbell looks for a version to check. Each look walks the
/// observed panes under the lock, so it is not done on every pass.
pub(super) const CHECK_EVERY_MS: u64 = 5_000;

/// A composer the doorbell may read, with the facts the read is made for.
pub(crate) struct Read {
    pub actor: Actor,
    pub raw_pane_id: String,
    /// The adapter id.
    pub agent: &'static str,
    pub composer: Composer,
    pub version: String,
    /// The input time of the draft episode a release is read for, or the
    /// prompt hook time of the moment a check is read at.
    pub input_at_unix_ms: u64,
}

/// What the reader keeps between doorbell passes. Each map is keyed by a pane
/// and pruned to the panes of the pass (`retain`) or of the observations, so
/// it is bounded by the open letters and the observation limit.
#[derive(Default)]
pub(crate) struct Reader {
    /// The draft episode each pane's composer was read for.
    read: HashMap<String, u64>,
    /// Why a pane's draft episode is not read, logged once per change.
    skipped: HashMap<String, (u64, &'static str)>,
    /// The prompt hook moment each pane was tried at for a version check.
    attempted: HashMap<String, u64>,
    next_check_at: u64,
}

impl Reader {
    /// Forgets the panes that have no pending letter any more.
    pub(crate) fn retain(&mut self, panes: impl Fn(&str) -> bool) {
        self.read.retain(|pane, _| panes(pane));
        self.skipped.retain(|pane, _| panes(pane));
    }

    /// Reads the composer of `recipient`, held on a possible draft, once per
    /// draft episode, and releases the draft when the screen shows the
    /// composer empty. Returns whether a Herdr call was made, which counts
    /// against the doorbell's per-pass budget.
    pub(crate) fn release(&mut self, owner: &Mutex<Runtime>, recipient: &Actor, now: u64) -> bool {
        let Some((episode, read)) = owner
            .lock()
            .ok()
            .and_then(|guard| guard.composer_release_due(recipient, now))
        else {
            return false;
        };
        let pane = &recipient.pane_id;
        if self.read.get(pane) == Some(&episode) {
            return false;
        }
        let read = match read {
            Ok(read) => read,
            Err(reason) => {
                if self.skipped.insert(pane.clone(), (episode, reason)) != Some((episode, reason)) {
                    crate::diagnostic!(json!({"component":"delivery","kind":"composer.not_read",
                        "pane_id":pane,"agent":recipient.kind,"reason":reason}));
                }
                return false;
            }
        };
        self.read.insert(pane.clone(), episode);
        let verdict = screen(owner, &read);
        crate::diagnostic!(json!({"component":"delivery","kind":"composer.read",
            "pane_id":pane,"agent":read.agent,"version":read.version,
            "verdict":verdict.map_or_else(|code| code, Verdict::code)}));
        if verdict == Ok(Verdict::Empty)
            && let Ok(mut guard) = owner.lock()
        {
            guard.release_empty_composer(&read.actor, read.input_at_unix_ms, now);
        }
        true
    }

    /// Checks one agent version's composer declaration, at most every
    /// `CHECK_EVERY_MS`, at a moment the composer is known empty. Returns
    /// whether a Herdr call was made.
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
            .insert(read.actor.pane_id.clone(), read.input_at_unix_ms);
        let verdict = screen(owner, &read);
        crate::diagnostic!(
            json!({"component":"delivery","kind":"composer.version_checked",
            "pane_id":read.actor.pane_id,"agent":read.agent,"version":read.version,
            "verdict":verdict.map_or_else(|code| code, Verdict::code)})
        );
        // A failed read says nothing about the screen, so the version waits
        // for its next known-empty moment.
        if let Ok(verdict) = verdict
            && let Ok(mut guard) = owner.lock()
        {
            guard.record_composer_check(read.agent, &read.version, verdict == Verdict::Empty);
        }
        true
    }
}

/// Reads the pane's visible screen with styling and judges it. The connector
/// is looked up under the lock; the read itself runs off it.
fn screen(owner: &Mutex<Runtime>, read: &Read) -> Result<Verdict, &'static str> {
    let connector = owner
        .lock()
        .ok()
        .and_then(|guard| guard.delivery_connector(&read.actor.device_id))
        .ok_or("pane_unavailable")?;
    judge(connector.as_ref(), read)
}

fn judge(connector: &dyn ApiConnector, read: &Read) -> Result<Verdict, &'static str> {
    let parameters =
        wire::composer_screen_params(&read.raw_pane_id).map_err(|_| "herdr_parameters")?;
    let answer = request_small_response(connector, "pane.read", parameters, READ_TIMEOUT)
        .map_err(|_| "read_failed")?;
    let screen = wire::pane_text(answer).map_err(|_| "herdr_format")?;
    // A cut screen may have lost the composer's rows, which no verdict
    // could tell from an empty composer.
    if screen.truncated {
        return Err("read_truncated");
    }
    Ok(read.composer.judge(&screen.text))
}
