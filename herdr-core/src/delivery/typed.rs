//! Input Hide typed into an agent, and the agent's own record that it took
//! it (`docs/delivery.md`, Input an agent did not take).
//!
//! Herdr can report an agent ready for input while a screen of the agent's
//! own (a hooks review, an update notice, a trust question) takes whatever is
//! typed, so a successful `agent.prompt` proves only that the text reached
//! the pane. Input is taken once the agent itself records it: its prompt
//! hook's digest of the submitted prompt (Claude Code, Codex, Grok), or the user
//! message in its session file (the other agents). Nothing here reads a
//! screen; the judgement is the record, the pane's rest and the input Hide
//! routed to it.

use serde::{Deserialize, Serialize};

use crate::agent_state::AgentActivity;
use crate::runtime::delivery::Observation;

/// The Hide role that tells a parent, or a letter's sender, that typed input
/// was not taken.
pub(crate) const INPUT_ROLE: &str = "input";

/// How long an agent at rest may go without its record of typed input before
/// the input is unconfirmed. This caps a wait for a record; it judges no
/// screen. A prompt hook records at the submission itself, and a session read
/// lands within seconds of the agent's next state change, both well inside
/// it (measured per agent, `docs/delivery.md`); an agent that records only
/// after its answer has worked meanwhile, and the wait counts from its rest.
pub(crate) const RECORD_WAIT_MS: u64 = 15_000;

/// How long a fresh read of the agent's session, asked once the record wait
/// ran out, has to land before the input is judged unconfirmed. An agent can
/// write the prompt to its session file after the read its stop asked for
/// (agy 1.4.8 on Windows, measured 2026-10-31: the brief it took and
/// answered was not in the read at its stop); the worker reads within a
/// tick of the ask. It caps a wait for a record, like `RECORD_WAIT_MS`.
pub(crate) const REREAD_WAIT_MS: u64 = 5_000;

/// Typings of one input at most: the first and two more, each only after
/// someone answered the agent's screen. Past them the input waits for the
/// operator with the reason.
pub(crate) const INPUT_ATTEMPTS: u8 = 3;

/// How long a parent told of unconfirmed input has to clear its child's
/// screen before the child's row shows it to the operator.
pub(crate) const PARENT_GRACE_MS: u64 = 2 * 60_000;

/// Where one typed input stands in the ledger: a spawn's brief keeps it on
/// its record. A letter keeps the same facts in its own fields.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedInput {
    /// Typings reserved, each saved before the text is typed, so a typing
    /// interrupted by a crash still counts and is never repeated blindly.
    pub attempts: u8,
    /// When the last typing was reserved.
    pub typed_at_unix_ms: u64,
    /// When the first typing was reserved, 0 on a record an earlier build
    /// wrote: an agent's record of any typing is written after this.
    #[serde(default, skip_serializing_if = "is_zero_ms")]
    pub first_typed_at_unix_ms: u64,
    /// When the agent's own record of the input arrived; it ends the input
    /// whichever typing it records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_at_unix_ms: Option<u64>,
    /// The typing the unconfirmed notice was sent for, 0 for none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub noticed: u8,
    #[serde(default, skip_serializing_if = "is_zero_ms")]
    pub noticed_at_unix_ms: u64,
    /// The input rode the agent's start as its prompt argument
    /// (`AgentApi::LaunchPrompt`): the CLI holds it through its own start
    /// screens, so it is never typed or typed again, and until the agent's
    /// record of it appears it is not yet taken (`delivery::ladder`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub launch: bool,
}

impl TypedInput {
    /// When the first typing was reserved; the last one on a record that does
    /// not know it, which is never earlier than the first.
    pub(crate) fn first_typed_at(&self) -> u64 {
        if self.first_typed_at_unix_ms == 0 {
            self.typed_at_unix_ms
        } else {
            self.first_typed_at_unix_ms
        }
    }
}

fn is_zero(value: &u8) -> bool {
    *value == 0
}

fn is_zero_ms(value: &u64) -> bool {
    *value == 0
}

/// What the record and the pane say about one typed input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Standing {
    /// The agent recorded it.
    Confirmed,
    /// Not typed yet, typed too recently, or the agent works or is not
    /// observed: no record is owed yet.
    Waiting,
    /// The core has no turn for the pane's current read: the read is
    /// stale, or the pass that observed the pane has not laid the hook's and
    /// the session read's facts yet (`Observation::projected_turn`). Nothing
    /// moves on it, and the last verdict stands (`Runtime::input_standing`).
    /// A state ends it, never a timer: the next agent list, asked every
    /// second while Herdr's stream stands, lays the turn of a fresh read; a
    /// stale read ends with the list that confirms the agents again, and
    /// until then the row shows the pane's activity unknown; a pane Herdr
    /// no longer lists leaves the observations and takes its input with it.
    Unsettled,
    /// Typed, and the agent rested without its record for
    /// [`RECORD_WAIT_MS`] since `since`.
    Unconfirmed { since: u64 },
}

/// Where `input` stands by `observation` of the pane it was typed into.
/// Whether the agent works is the core's turn (`agent_state::turn_projection`,
/// the answer the row and the doorbell read), never Herdr's own status: an
/// agent the turn reads at rest, or in a state no source names, rests.
pub(crate) fn standing(
    input: &TypedInput,
    observation: Option<&Observation>,
    now: u64,
) -> Standing {
    if input.confirmed_at_unix_ms.is_some() {
        return Standing::Confirmed;
    }
    let Some(observation) = observation else {
        return Standing::Waiting;
    };
    if input.attempts == 0 {
        return Standing::Waiting;
    }
    let Some(projected) = observation.projected_turn() else {
        return Standing::Unsettled;
    };
    if projected.turn.activity == AgentActivity::Working {
        return Standing::Waiting;
    }
    let rest_since = input
        .typed_at_unix_ms
        .max(projected.changed_at(observation));
    if now.saturating_sub(rest_since) >= RECORD_WAIT_MS {
        Standing::Unconfirmed { since: rest_since }
    } else {
        Standing::Waiting
    }
}

/// Whether unconfirmed `input` may be typed again now: someone answered the
/// agent's screen since the last typing (`Observation::answered_at_unix_ms`:
/// the operator's keys, a phone reply, or a parent's `hide pane send` that
/// landed whole, never one that stopped partway), input has been quiet
/// for the quiet period, the agent rests (`composer_readable`), and typings
/// are left. Text and Enter on a choice screen can pick an option, so an
/// unanswered screen is never typed into again; the composer is read empty
/// right before the typing as well (`composer::forced_clear`).
pub(crate) fn retype_due(input: &TypedInput, observation: &Observation, now: u64) -> bool {
    matches!(
        standing(input, Some(observation), now),
        Standing::Unconfirmed { .. }
    ) && input.attempts < INPUT_ATTEMPTS
        && observation.answered_at_unix_ms > input.typed_at_unix_ms
        && composer_readable(observation, now)
}

/// Whether the composer of an agent whose input is unconfirmed may be read
/// now: the core's turn reads it at rest with nothing of its own waiting,
/// Herdr knows its state and lets keys in (the doorbell's gate,
/// `doorbell::unheld_turn` and `doorbell::takes_keys`), and nobody typed
/// there for the quiet period, so what it shows is what was left there.
pub(crate) fn composer_readable(observation: &Observation, now: u64) -> bool {
    super::doorbell::unheld_turn(observation)
        .is_ok_and(|projected| projected.turn.activity == AgentActivity::Stopped)
        && super::doorbell::takes_keys(observation).is_ok()
        && now.saturating_sub(observation.last_input_at_unix_ms) >= super::INPUT_QUIET_MS
}

/// Whether the parent (or sender) of unconfirmed `input` is still owed its
/// notice of the latest typing.
pub(crate) fn notice_due(input: &TypedInput) -> bool {
    input.noticed < input.attempts
}

/// Whether unconfirmed `input` reaches the operator: no parent can clear it,
/// the parent had its grace, or no typing is left.
pub(crate) fn shown_to_operator(input: &TypedInput, has_parent: bool, now: u64) -> bool {
    !has_parent
        || input.attempts >= INPUT_ATTEMPTS
        || (input.noticed > 0 && now.saturating_sub(input.noticed_at_unix_ms) >= PARENT_GRACE_MS)
}

/// What was typed, for the notice and the row: a spawn's brief or a letter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    Brief,
    Letter,
}

/// Why an agent's own record of typed input cannot be had: its prompt hook
/// is not connected (Hide's hook missing, outdated or switched off for it,
/// the pane's Not connected), or Hide has no read of its session file.
pub(crate) const HOOKS_NOT_CONNECTED: &str = "hooks_not_connected";
pub(crate) const SESSION_UNREAD: &str = "session_unread";

/// The notice's body: its first line says what happened and the one command
/// to start with, since the operator reads it in the wake summary. With no
/// source for the agent's record (`source` names why), the input is not
/// called lost and is not typed again.
pub(crate) fn notice_body(
    kind: InputKind,
    agent: &str,
    pane: &str,
    letter: Option<&str>,
    source: Result<(), &str>,
    launch: bool,
) -> String {
    let what = match (kind, letter) {
        (InputKind::Brief, _) => "its brief".to_owned(),
        (InputKind::Letter, Some(id)) => format!("your letter {id}"),
        (InputKind::Letter, None) => "your letter".to_owned(),
    };
    // The operator's decision (2026-11-01): an agent may answer another's
    // screen, approvals included; Hide does not judge consent, and the
    // operator sets how much agents approve in Settings > Agents.
    let trust = "Answering a trust or approval screen for it is your call: Hide does not check \
                 consent, and the operator sets how much agents may approve in Settings > Agents. \
                 Leave a login to the operator.";
    match source {
        // The agent's start holds a launch prompt through its own screens
        // and submits it after them, so nothing types it again.
        Ok(()) if launch => format!(
            "{agent} has not taken {what} yet: read its screen with `hide pane read {pane}`.\n\
             Its start holds the brief and submits it once its screen is answered; \
             `hide agent show` names what it waits on. Answer the screen with \
             `hide pane send {pane} --key <name>` (esc, enter, up, down, tab, ...) or with text \
             and `--enter`; Hide never types the brief, so answering cannot send it twice.\n{trust}"
        ),
        Ok(()) => format!(
            "{agent} has not confirmed {what}: read its screen with `hide pane read {pane}`.\n\
             Something on its screen may have taken the typed text; `hide agent show` names what \
             it waits on. Answer the screen with `hide pane send {pane} --key <name>` (esc, enter, \
             up, down, tab, ...) or with text and `--enter`; Hide types it again once the agent \
             is ready.\n{trust}"
        ),
        Err(reason) => {
            let why = if reason == HOOKS_NOT_CONNECTED {
                "its hooks are not connected"
            } else {
                "Hide cannot read its session"
            };
            format!(
                "{agent} cannot confirm {what} ({why}): read its screen with `hide pane read {pane}`.\n\
                 Hide does not type it again, since the agent may have taken it.\n{trust}"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::Actor;
    use crate::delivery::doorbell::Turn;

    fn observation(status: &str, changed: u64, input: u64) -> Observation {
        Observation {
            actor: Actor {
                pane_id: "w1:p2".into(),
                name: "child".into(),
                kind: "codex".into(),
                device_id: "node".into(),
                session: None,
            },
            raw_pane_id: "w1:p2".into(),
            observed_since_unix_ms: 0,
            terminal_id: Some("term-1".into()),
            status: status.into(),
            telemetry_stale: false,
            state_change_seq: Some(4),
            status_changed_at_unix_ms: changed,
            last_input_at_unix_ms: input,
            last_submit_at_unix_ms: 0,
            entered_working_at_unix_ms: 0,
            released_input_at_unix_ms: 0,
            input_generation: 0,
            pane_send_generation: None,
            answered_at_unix_ms: input,
            observed_at_unix_ms: 0,
            hook_submit_at_unix_ms: 0,
            session: None,
            thread: None,
            host_scope: None,
            turn: Turn::NotReported,
            projected: None,
            observed_generation: 0,
            unreadable: None,
        }
        // The core's turn, from Herdr's state alone, as the pass lays it.
        .laid(None, None)
    }

    fn typed(at: u64) -> TypedInput {
        TypedInput {
            attempts: 1,
            typed_at_unix_ms: at,
            ..TypedInput::default()
        }
    }

    #[test]
    fn typed_input_is_unconfirmed_only_after_the_agent_rested_without_its_record() {
        let input = typed(1_000);
        let idle = observation("idle", 0, 0);
        assert_eq!(
            standing(&input, Some(&idle), 1_000 + RECORD_WAIT_MS - 1),
            Standing::Waiting
        );
        assert_eq!(
            standing(&input, Some(&idle), 1_000 + RECORD_WAIT_MS),
            Standing::Unconfirmed { since: 1_000 }
        );
        // An agent that works on it owes no record yet, however long.
        let working = observation("working", 2_000, 0);
        assert_eq!(
            standing(&input, Some(&working), 10 * RECORD_WAIT_MS),
            Standing::Waiting
        );
        // Its rest counts from the end of that work.
        let rested = observation("done", 50_000, 0);
        assert_eq!(
            standing(&input, Some(&rested), 50_000 + RECORD_WAIT_MS - 1),
            Standing::Waiting
        );
        // A record ends it whatever the pane does.
        let confirmed = TypedInput {
            confirmed_at_unix_ms: Some(2_000),
            ..input.clone()
        };
        assert_eq!(
            standing(&confirmed, Some(&idle), u64::MAX),
            Standing::Confirmed
        );
        assert_eq!(standing(&input, None, u64::MAX), Standing::Waiting);
    }

    #[test]
    fn unconfirmed_input_is_typed_again_only_after_someone_answered_the_screen() {
        let input = typed(1_000);
        let now = 100_000;
        // Nobody touched the pane since the typing: never typed blindly again.
        assert!(!retype_due(&input, &observation("idle", 0, 500), now));
        // Keys reached the pane, but too recently.
        assert!(!retype_due(&input, &observation("idle", 0, now - 1), now));
        let answered = observation("idle", 0, now - crate::delivery::INPUT_QUIET_MS);
        assert!(retype_due(&input, &answered, now));
        // No typing left.
        let spent = TypedInput {
            attempts: INPUT_ATTEMPTS,
            ..input.clone()
        };
        assert!(!retype_due(&spent, &answered, now));
        // A blocked menu is no rest to type into.
        let blocked = observation("blocked", 0, now - crate::delivery::INPUT_QUIET_MS);
        assert!(!retype_due(&input, &blocked, now));
    }

    #[test]
    fn the_operator_sees_unconfirmed_input_once_no_parent_can_clear_it() {
        let mut input = typed(0);
        assert!(shown_to_operator(&input, false, 0));
        assert!(!shown_to_operator(&input, true, u64::MAX));
        input.noticed = 1;
        input.noticed_at_unix_ms = 10;
        assert!(!shown_to_operator(&input, true, 10 + PARENT_GRACE_MS - 1));
        assert!(shown_to_operator(&input, true, 10 + PARENT_GRACE_MS));
        input.attempts = INPUT_ATTEMPTS;
        assert!(shown_to_operator(&input, true, 11));
    }
}
