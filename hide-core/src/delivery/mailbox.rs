pub use hide_node_link::panes::PullPoint;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::answer::{Acknowledged, Confirmed, Folded, Inbox};
use super::ledger::{Ledger, Letter, State};
use super::{
    Actor, BODY_LIMIT, BUNDLE_LETTERS, HOOK_LIMIT, LETTER_LIMIT, OPEN_LIMIT, SHOWN_PER_SENDER,
};
use crate::model::InterfaceLanguage;

// One per request, made and consumed by its caller and never kept in bulk,
// so the size of the agent commands is not worth a box.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Agents {
        command: crate::coordination::Command,
    },
    Send {
        target: String,
        intent: String,
        body: String,
        #[serde(default = "request_kind")]
        kind: String,
        /// `hide request send --now`: delivered at the recipient's next safe
        /// gap while it works, without the mid-turn wait (PRD
        /// letters-to-busy-agents D-05).
        #[serde(default)]
        now: bool,
        /// `hide request send --force`: typed into the recipient's input now,
        /// even while it works, once its composer was read empty (PRD
        /// engine-bundle-1 D-14); see `Letter::forced`.
        #[serde(default)]
        force: bool,
    },
    Reply {
        id: String,
        intent: String,
        body: String,
    },
    Ack {
        id: String,
    },
    /// `hide inbox --ack <bundle>`: acknowledges, as one receipt, exactly
    /// the pending letters `hide inbox` listed, folded ones included, which
    /// `bundle` names ([`bundle_token`]); a set that changed since is refused
    /// `bundle_changed`, and a letter that came after it is not taken.
    AckBundle {
        bundle: String,
    },
    Cancel {
        id: String,
    },
    Show {
        id: String,
    },
    Inbox {
        /// Set by a `hide` that reads the bundle ([`Bundling`]).
        #[serde(default)]
        bundle: bool,
    },
    /// A runtime hook asks what to hand the agent. `prompt_digest` is the
    /// digest of the submitted prompt (`hide_agent_hooks::delivery::
    /// prompt_digest`): the turn whose prompt is the line the doorbell typed
    /// for one of the caller's pending letters receives the letter bodies,
    /// while any other prompt receives at most a count.
    Pull {
        #[serde(default)]
        prompt_digest: Option<String>,
        /// The session id the hook's runtime reported. The hook only counts
        /// as a submission in the pane when it is the pane's own session.
        #[serde(default)]
        session: Option<String>,
        /// The hook point whose declared intake path receives the letters.
        #[serde(default)]
        point: PullPoint,
        /// Set by a `hide` that confirms a whole bundle ([`Bundling`]).
        #[serde(default)]
        bundle: bool,
    },
    Confirm {
        ids: Vec<String>,
    },
    WatchStart {
        target: String,
        observer: Option<String>,
        actor: Option<String>,
    },
    WatchStop {
        id: String,
    },
    WatchAssign {
        id: String,
        observer: String,
        actor: Option<String>,
        expected_generation: Option<u64>,
        #[serde(default)]
        approval: Option<String>,
    },
    WatchList,
}

impl Command {
    /// The command a client sent. A prompt hook of an earlier build names the
    /// bell with `bell: true`, which proves nothing now that the bell is the
    /// line the doorbell kept; the field is dropped, so that hook still
    /// receives a count line, or in an agent no bell rings for its letters,
    /// until the kit beside it is replaced.
    pub fn from_wire(mut value: Value) -> serde_json::Result<Self> {
        if value["op"] == "pull"
            && let Some(fields) = value.as_object_mut()
        {
            fields.remove("bell");
        }
        serde_json::from_value(value)
    }
}

fn request_kind() -> String {
    "request".into()
}

pub(crate) fn existing_intent<'a>(
    ledger: &'a Ledger,
    sender: &Actor,
    intent: &str,
    now: u64,
) -> Option<&'a Letter> {
    ledger.letters.iter().find(|letter| {
        letter.sender.same_identity(sender) && letter.intent == intent && letter.retained(now)
    })
}

/// The letter a send under `intent` retries, if its sender already holds one.
///
/// A retry is the same letter again: the recipient (`to_recipient`, which
/// each caller words in the terms it holds), the kind and the body are those
/// the stored letter has. A report's stored body also carries the list of
/// Views its sender opened (`list_opened_views`), added after the sender
/// wrote it, so a report is the same letter when its stored body begins with
/// the body sent. Another letter under the same intent is `intent_conflict`,
/// refused before anything is stored, so a letter is never answered with an
/// earlier one that went elsewhere.
pub(crate) fn retried_send<'a>(
    ledger: &'a Ledger,
    sender: &Actor,
    intent: &str,
    kind: &str,
    body: &str,
    to_recipient: impl FnOnce(&Actor) -> bool,
    now: u64,
) -> Result<Option<&'a Letter>, String> {
    let Some(letter) = existing_intent(ledger, sender, intent, now) else {
        return Ok(None);
    };
    let same_body = if kind == "report" {
        letter.body.starts_with(body)
    } else {
        letter.body == body
    };
    if letter.kind == kind && same_body && to_recipient(&letter.recipient) {
        Ok(Some(letter))
    } else {
        Err("intent_conflict".into())
    }
}

/// Where the finished letter that finished first stands: the one a full
/// ledger drops to make room. An open letter is never a candidate.
fn oldest_finished(ledger: &Ledger) -> Option<usize> {
    ledger
        .letters
        .iter()
        .enumerate()
        .filter(|(_, letter)| !letter.open())
        .min_by_key(|(index, letter)| {
            (
                letter
                    .finished_at_unix_ms
                    .unwrap_or(letter.created_at_unix_ms),
                *index,
            )
        })
        .map(|(index, _)| index)
}

#[expect(
    clippy::too_many_arguments,
    reason = "The private transaction validates each immutable envelope field before persistence"
)]
pub(crate) fn send(
    ledger: &mut Ledger,
    sender: &Actor,
    recipient: &Actor,
    intent: &str,
    body: &str,
    kind: &str,
    reply_to: Option<String>,
    now: u64,
) -> Result<Letter, String> {
    sender.require_native_identity()?;
    recipient.require_native_identity()?;
    if !sender.valid() || !recipient.valid() {
        return Err("invalid_actor".into());
    }
    if !super::valid_key(intent) {
        return Err("invalid_intent".into());
    }
    if !matches!(
        kind,
        "request" | "block" | "report" | "progress" | "reply" | "watch"
    ) {
        return Err("invalid_kind".into());
    }
    if let Some(letter) = retried_send(
        ledger,
        sender,
        intent,
        kind,
        body,
        |stored| stored.same_identity(recipient),
        now,
    )? {
        return Ok(letter.clone());
    }
    let open = ledger.letters.iter().filter(|letter| letter.open()).count();
    // A full total makes room by dropping the oldest finished letter; it is
    // refused only when every letter is still open.
    let full = ledger.letters.len() >= LETTER_LIMIT;
    let oldest_finished = full.then(|| oldest_finished(ledger)).flatten();
    let refused = if body.len() > BODY_LIMIT {
        Some("body")
    } else if full && oldest_finished.is_none() {
        Some("total")
    } else if open >= OPEN_LIMIT {
        Some("open")
    } else {
        None
    };
    if let Some(cap) = refused {
        // The caller sees only `capacity`; the log says which cap fired.
        crate::diagnostic!(
            json!({"component":"delivery","kind":"letter.capacity_refused",
            "cap":cap,"sender_pane":sender.pane_id,"recipient_pane":recipient.pane_id,
            "body_bytes":body.len(),"letters":ledger.letters.len(),"open":open})
        );
        return Err("capacity".into());
    }
    if body.trim().is_empty() {
        return Err("body_required".into());
    }
    let sequence = ledger.next_id;
    ledger.next_id = sequence.checked_add(1).ok_or("capacity")?;
    if let Some(index) = oldest_finished {
        ledger.letters.remove(index);
    }
    let letter = Letter {
        id: format!("letter-{sequence}"),
        intent: intent.to_owned(),
        sender: crate::coordination::named(ledger, sender),
        recipient: crate::coordination::named(ledger, recipient),
        kind: kind.to_owned(),
        body: body.to_owned(),
        state: State::Pending,
        hook_confirmed: Some(false),
        waiting_answer: matches!(kind, "request" | "block"),
        reply_to,
        created_at_unix_ms: now,
        finished_at_unix_ms: None,
        bell_errors: 0,
        bell_sent: false,
        bell_attempts: Some(0),
        human_notified: false,
        watch_warning: None,
        answer_wait_ended: None,
        bell_line: None,
        bell_turn: None,
        handed: false,
        handed_at_unix_ms: None,
        immediate: false,
        forced: false,
        tool_handovers: 0,
        bell_typed_at_unix_ms: None,
        bell_route: Default::default(),
        input_noticed: 0,
        input_noticed_at_unix_ms: 0,
        bell_failure: None,
        bell_ended: Some(0),
        // A letter to a machine with a clock waits on that clock from now;
        // the store gives one to a machine a letter waits on.
        intake_from: ledger
            .node_clocks
            .get(&recipient.device_id)
            .map(|clock| clock.reading(now)),
    };
    ledger.letters.push(letter.clone());
    Ok(letter)
}

pub fn apply(
    ledger: &mut Ledger,
    actor: &Actor,
    target: Option<&Actor>,
    command: &Command,
    now: u64,
) -> Result<Value, String> {
    apply_in(
        ledger,
        actor,
        target,
        command,
        now,
        InterfaceLanguage::English,
        &[],
    )
}

/// [`apply`], with what a handover shows worded in `language` and `also_reads`
/// the senders whose letters `Show` answers beside the caller's own
/// (`Authority::also_reads`).
pub fn apply_in(
    ledger: &mut Ledger,
    actor: &Actor,
    target: Option<&Actor>,
    command: &Command,
    now: u64,
    language: InterfaceLanguage,
    also_reads: &[Exchange],
) -> Result<Value, String> {
    actor.require_native_identity()?;
    match command {
        // The runtime judged a repeated send's recipient from the key it
        // named and hands no target; a target handed here is compared whole.
        Command::Send {
            intent, kind, body, ..
        } => {
            if let Some(letter) = retried_send(
                ledger,
                actor,
                intent,
                kind,
                body,
                |stored| target.is_none_or(|target| target.same_identity(stored)),
                now,
            )? {
                return Ok(json!(letter));
            }
        }
        Command::Reply { intent, .. } => {
            if let Some(letter) = existing_intent(ledger, actor, intent, now) {
                return Ok(json!(letter));
            }
        }
        _ => {}
    }
    match command {
        Command::Send {
            intent,
            body,
            kind,
            now: immediate,
            force,
            ..
        } => {
            let mut letter = send(
                ledger,
                actor,
                target.ok_or("target_unavailable")?,
                intent,
                body,
                kind,
                None,
                now,
            )?;
            if (*immediate || *force)
                && let Some(stored) = ledger
                    .letters
                    .iter_mut()
                    .find(|stored| stored.id == letter.id)
            {
                stored.immediate = true;
                stored.forced = *force;
                letter = stored.clone();
            }
            Ok(json!(letter))
        }
        Command::Reply { id, intent, body } => {
            let original = authorized(ledger, actor, id)?.clone();
            if !original.recipient.same_identity(actor) {
                return Err("recipient_required".into());
            }
            let reply = send(
                ledger,
                actor,
                &original.sender,
                intent,
                body,
                "reply",
                Some(id.clone()),
                now,
            )?;
            let original = ledger
                .letters
                .iter_mut()
                .find(|letter| letter.id == *id)
                .ok_or("letter_unavailable")?;
            original.waiting_answer = false;
            if original.state != State::Pending {
                original.finished_at_unix_ms.get_or_insert(now);
            }
            Ok(json!(reply))
        }
        Command::Ack { id } | Command::Cancel { id } => {
            let original = authorized(ledger, actor, id)?;
            if matches!(command, Command::Ack { .. }) && !original.recipient.same_identity(actor) {
                return Err("recipient_required".into());
            }
            if matches!(command, Command::Cancel { .. }) && !original.sender.same_identity(actor) {
                return Err("sender_required".into());
            }
            let letter = ledger
                .letters
                .iter_mut()
                .find(|letter| letter.id == *id)
                .ok_or("letter_unavailable")?;
            let ended = if matches!(command, Command::Ack { .. }) {
                acknowledge(letter, now)
            } else {
                letter.state = State::Cancelled;
                letter.waiting_answer = false;
                if !letter.open() {
                    letter.finished_at_unix_ms.get_or_insert(now);
                }
                None
            };
            let letter = letter.clone();
            end_report_watches(ledger, ended);
            Ok(json!(letter))
        }
        Command::AckBundle { bundle } => {
            let (newest, _) = bundle.rsplit_once(BUNDLE_MARK).ok_or("invalid_request")?;
            if !authorized(ledger, actor, newest)?
                .recipient
                .same_identity(actor)
            {
                return Err("recipient_required".into());
            }
            // The ledger keeps letters in the order they were sent, so a
            // letter that came after the bundle stands after its newest, and
            // the token proves the rest are exactly the letters listed.
            let end = ledger
                .letters
                .iter()
                .position(|letter| letter.id == newest)
                .ok_or("letter_unavailable")?;
            let listed = |letter: &Letter| {
                letter.recipient.same_identity(actor) && letter.state == State::Pending
            };
            let ids: Vec<&str> = ledger.letters[..=end]
                .iter()
                .filter(|letter| listed(letter))
                .map(|letter| letter.id.as_str())
                .collect();
            if bundle_token(&ids).as_deref() != Some(bundle.as_str()) {
                return Err("bundle_changed".into());
            }
            let mut reports = Vec::new();
            let mut acknowledged = 0;
            for letter in ledger.letters[..=end]
                .iter_mut()
                .filter(|letter| listed(letter))
            {
                reports.extend(acknowledge(letter, now));
                acknowledged += 1;
            }
            // One receipt for the whole bundle, with the watches it ends.
            for report in reports {
                end_report_watches(ledger, Some(report));
            }
            Ok(json!(Acknowledged {
                bundle: bundle.clone(),
                acknowledged,
            }))
        }
        Command::Show { id } => Ok(json!(show(ledger, actor, also_reads, id)?)),
        Command::Inbox { bundle: true } => Ok(json!(inbox(ledger, actor))),
        Command::Inbox { bundle: false } => Ok(legacy_inbox(ledger, actor)),
        // A tool event's ask is answered by the worker with the pane's own
        // observation (`tool_pull`); with none, nothing is due.
        Command::Pull {
            point: PullPoint::Tool | PullPoint::TurnEnd,
            ..
        } => Ok(json!(Intake::default())),
        Command::Pull {
            prompt_digest,
            session,
            ..
        } if hide_agent_adapter::adapter(&actor.kind)
            .and_then(|row| row.prompt_hook)
            .is_some_and(|hook| !hook.context) =>
        {
            let session = session
                .as_deref()
                .filter(|session| super::valid_session(session))
                .and_then(crate::engine::tokens::session_digest)
                .filter(|session| actor.session.as_ref() == Some(session))
                .ok_or("session_mismatch")?;
            let digest = prompt_digest
                .as_deref()
                .filter(|digest| rang(ledger, actor, digest));
            // A retried hook keeps the original batch, even if a new letter
            // arrived between the submission and its retry.
            if digest.is_some_and(|digest| {
                ledger.letters.iter().any(|letter| {
                    letter.recipient.same_identity(actor)
                        && letter
                            .bell_turn
                            .as_ref()
                            .is_some_and(|turn| turn.session == session && turn.digest == digest)
                })
            }) {
                return Ok(json!(Intake::default()));
            }
            for letter in ledger
                .letters
                .iter_mut()
                .filter(|letter| letter.recipient.same_identity(actor))
            {
                // Every own-session submission replaces the previous turn's
                // claim. Letters arriving later still obey the normal wait.
                letter.bell_turn = digest.filter(|_| letter.awaiting_intake()).map(|digest| {
                    super::ledger::BellTurn {
                        session: session.clone(),
                        digest: digest.to_owned(),
                    }
                });
            }
            Ok(json!(Intake::default()))
        }
        Command::Pull {
            prompt_digest: Some(digest),
            ..
        } if rang(ledger, actor, digest) => {
            let intake = pull_in(ledger, actor, language, Bundling::of(command))?;
            // Handed over now, whether or not the hook's confirmation, a
            // second process inside its last 400 ms, ever arrives.
            mark_handed(ledger, &intake, now);
            Ok(json!(intake))
        }
        // An agent no bell rings for takes its letters on its next prompt,
        // whoever typed it: no turn of Hide's own will ever bring them.
        Command::Pull { .. } if next_prompt_letters(&actor.kind) => Ok(json!(pull_in(
            ledger,
            actor,
            language,
            Bundling::of(command)
        )?)),
        Command::Pull { .. } => Ok(json!(operator_prompt_intake(ledger, actor, now)?)),
        Command::Confirm { ids } => {
            recipient_of_all(ledger, actor, ids)?;
            let mut reports = Vec::new();
            for id in ids {
                let letter = ledger
                    .letters
                    .iter_mut()
                    .find(|letter| letter.id == *id)
                    .ok_or("letter_unavailable")?;
                reports.extend(record_intake(letter, now));
            }
            // Delivery and ending the matching watch are one durable transaction.
            for report in reports {
                end_report_watches(ledger, Some(report));
            }
            Ok(json!(Confirmed {
                confirmed: ids.clone()
            }))
        }
        Command::Agents { .. }
        | Command::WatchStart { .. }
        | Command::WatchStop { .. }
        | Command::WatchAssign { .. }
        | Command::WatchList => Err("watch_command_required".into()),
    }
}

/// The letters a pane's hooks handed to `recipient` and could not confirm
/// in time, which the node of the pane's machine handed over
/// (`hide_agent_hooks::owed`, `doorbell::Doorbell::settle_owed`): each that
/// is a letter to `recipient` still awaiting intake is taken in as the
/// hook's confirmation would have taken it. A record of another session,
/// which ended in the pane, confirms nothing. Returns the letters confirmed.
pub(crate) fn confirm_owed(
    ledger: &mut Ledger,
    recipient: &Actor,
    owed: &hide_agent_hooks::owed::Owed,
    now: u64,
) -> Vec<String> {
    let session = Some(owed.session.as_str())
        .filter(|session| super::valid_session(session))
        .and_then(crate::engine::tokens::session_digest);
    if session.is_none() || session != recipient.session {
        return Vec::new();
    }
    let mut confirmed = Vec::new();
    let mut reports = Vec::new();
    for letter in ledger.letters.iter_mut().filter(|letter| {
        letter.recipient.same_identity(recipient)
            && letter.awaiting_intake()
            && owed.ids.contains(&letter.id)
    }) {
        reports.extend(record_intake(letter, now));
        confirmed.push(letter.id.clone());
    }
    for report in reports {
        end_report_watches(ledger, Some(report));
    }
    confirmed
}

/// Refuses `ids` unless each names a letter to `actor`, and at most one
/// bundle's worth of them, before any of them changes.
fn recipient_of_all(ledger: &Ledger, actor: &Actor, ids: &[String]) -> Result<(), String> {
    if ids.len() > BUNDLE_LETTERS {
        return Err("capacity".into());
    }
    for id in ids {
        if !authorized(ledger, actor, id)?
            .recipient
            .same_identity(actor)
        {
            return Err("recipient_required".into());
        }
    }
    Ok(())
}

/// The recipient's acknowledgement of `letter`: only the recipient's own
/// pane and session may give one, so it is its receipt whatever its kind; a
/// letter already cancelled or expired has none to give. Returns the pair
/// whose report watch it ends.
fn acknowledge(letter: &mut Letter, now: u64) -> Option<(Actor, Actor)> {
    let mut ended = None;
    // An older build's acknowledgement may still await intake.
    if letter.awaiting_intake() || letter.state == State::Delivered {
        letter.state = State::Acknowledged;
        ended = record_intake(letter, now);
    }
    if !letter.open() {
        letter.finished_at_unix_ms.get_or_insert(now);
    }
    ended
}

/// Records that the recipient took the letter in. A report, the letter that
/// says the work is finished, returns the pair whose watch the intake ends;
/// a progress letter is an interim report and ends nothing.
fn record_intake(letter: &mut Letter, now: u64) -> Option<(Actor, Actor)> {
    if letter.intake_confirmed() {
        return None;
    }
    letter.hook_confirmed = Some(true);
    if matches!(
        letter.state,
        State::Pending | State::Undelivered | State::Expired
    ) {
        letter.state = State::Delivered;
    }
    if !letter.waiting_answer {
        letter.finished_at_unix_ms = Some(now);
    }
    (letter.kind == "report").then(|| (letter.sender.clone(), letter.recipient.clone()))
}

fn end_report_watches(ledger: &mut Ledger, report: Option<(Actor, Actor)>) {
    if let Some((sender, recipient)) = report {
        ledger.end_report_watch(&sender, &recipient);
    }
}

/// Letters from one agent to another that a third may read.
#[derive(Clone, Debug)]
pub struct Exchange {
    pub from: Actor,
    pub to: Actor,
}

/// The letter `id` for `actor` to read: one it sent or received, or one
/// in `also_reads`, the exchanges the caller may read beside those
/// (`Authority::also_reads`). This is the one place that decides who reads a
/// letter; taking one in, cancelling and answering it stay with
/// [`authorized`].
fn show<'a>(
    ledger: &'a Ledger,
    actor: &Actor,
    also_reads: &[Exchange],
    id: &str,
) -> Result<&'a Letter, String> {
    ledger
        .letters
        .iter()
        .find(|letter| {
            letter.id == id
                && (is_party(letter, actor)
                    || also_reads.iter().any(|read| {
                        letter.sender.same_identity(&read.from)
                            && letter.recipient.same_identity(&read.to)
                    }))
        })
        .ok_or_else(|| "letter_unavailable".into())
}

fn is_party(letter: &Letter, actor: &Actor) -> bool {
    letter.sender.same_identity(actor) || letter.recipient.same_identity(actor)
}

fn authorized<'a>(ledger: &'a Ledger, actor: &Actor, id: &str) -> Result<&'a Letter, String> {
    ledger
        .letters
        .iter()
        .find(|letter| letter.id == id && is_party(letter, actor))
        .ok_or_else(|| "letter_unavailable".into())
}

#[derive(Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Intake {
    pub context: String,
    pub ids: Vec<String>,
    pub remaining: usize,
}

/// Whether the kind's installed prompt hook can hand letters to the agent.
/// Any other agent reads them with `hide inbox` and acknowledges them, and a
/// Claude Code hook that runs inside one (Grok, OpenCode) must not count as
/// that agent having read anything.
pub(crate) fn prompt_hook(kind: &str) -> bool {
    hide_agent_adapter::adapter(kind)
        .is_some_and(|row| row.prompt_hook.is_some_and(|hook| hook.context))
}

/// Whether the kind's prompt hook hands letters over on any prompt: it has one
/// and is never belled (OpenCode, whose plugin confirms them only after
/// OpenCode stored the prompt that carries them).
pub(crate) fn next_prompt_letters(kind: &str) -> bool {
    prompt_hook(kind) && !super::doorbell::bell_target(kind)
}

/// Whether the submitted prompt, as its digest, is the line the doorbell
/// typed for one of the caller's pending letters: the line it kept on the
/// letter before typing it. Text a person typed, a bell line copied into
/// another pane or a line whose letter was already taken in is not.
fn rang(ledger: &Ledger, actor: &Actor, digest: &str) -> bool {
    ledger.letters.iter().any(|letter| {
        letter.state == State::Pending
            && letter.recipient.same_identity(actor)
            && letter
                .bell_line
                .as_deref()
                .is_some_and(|line| hide_agent_hooks::delivery::matches_prompt_digest(line, digest))
    })
}

/// Whether a bell is still coming for the letter. A letter whose three bells
/// are spent, or whose recipient is never belled, stays pending for
/// `hide inbox` and expires undelivered; the operator's own prompt promises
/// it nothing.
fn bell_pending(ledger: &Ledger, letter: &Letter, now: u64) -> bool {
    letter.state == State::Pending
        && !letter.handed
        && letter.within_deadline(&ledger.node_clocks, now)
        && letter.attempts() < super::ledger::BELLS
        && super::doorbell::bell_target(&letter.recipient.kind)
}

/// What the hook hands the agent when the operator's own prompt was
/// submitted: never a letter body, only a line counting the letters a bell
/// will still bring, so the operator's turn is not mixed with them.
fn operator_prompt_intake(ledger: &Ledger, actor: &Actor, now: u64) -> Result<Intake, String> {
    actor.require_native_identity()?;
    let waiting = ledger
        .letters
        .iter()
        .filter(|letter| letter.recipient.same_identity(actor) && bell_pending(ledger, letter, now))
        .count();
    let mut intake = Intake::default();
    if waiting > 0 && prompt_hook(&actor.kind) {
        intake.context = format!("Hide 편지 {waiting}통 대기 중, 이 턴이 끝난 뒤 전달\n");
    }
    Ok(intake)
}

/// Whether the kind's hook, plugin or extension takes letters at a tool event
/// while the agent works (PRD letters-to-busy-agents D-02, B3): declared on
/// the adapter; a kind with this path is belled only at rest.
pub(crate) fn tool_letters(kind: &str) -> bool {
    hide_agent_adapter::adapter(kind).is_some_and(|row| row.tool_letters.is_some())
}

/// How many times a tool event hands one letter over without a receipt
/// before it waits for the next prompt or `hide inbox`, as a letter does
/// after its three bells (PRD letters-to-busy-agents D-11).
pub(crate) const TOOL_HANDOVERS: u8 = 3;

#[cfg(test)]
pub(crate) fn tool_pull(
    ledger: &Ledger,
    actor: &Actor,
    working_since: Option<u64>,
    now: u64,
) -> Result<Intake, String> {
    gap_pull(
        ledger,
        actor,
        PullPoint::Tool,
        working_since,
        now,
        InterfaceLanguage::English,
        Bundling::PerSender,
    )
}

/// At a safe gap, take this session's bell batch immediately, then letters
/// whose wait during uninterrupted work is over. Both hooks share the caps.
pub(crate) fn gap_pull(
    ledger: &Ledger,
    actor: &Actor,
    point: PullPoint,
    working_since: Option<u64>,
    now: u64,
    language: InterfaceLanguage,
    bundling: Bundling,
) -> Result<Intake, String> {
    actor.require_native_identity()?;
    let supported = hide_agent_adapter::adapter(&actor.kind).is_some_and(|row| match point {
        PullPoint::Tool => row.tool_letters.is_some(),
        PullPoint::TurnEnd => row.turn_end_letters.is_some(),
        PullPoint::Prompt => false,
    });
    if !supported {
        return Ok(Intake::default());
    }
    let due: Vec<_> = ledger
        .letters
        .iter()
        .filter(|letter| {
            letter.recipient.same_identity(actor)
                && letter.awaiting_intake()
                && letter.within_deadline(&ledger.node_clocks, now)
                && letter.tool_handovers < TOOL_HANDOVERS
                && (letter
                    .bell_turn
                    .as_ref()
                    .is_some_and(|turn| actor.session.as_ref() == Some(&turn.session))
                    || (letter.state == State::Pending
                        && working_since.is_some_and(|since| {
                            super::doorbell::Patience::of(letter).due(since, now)
                        })))
        })
        .collect();
    Ok(render(due, language, bundling))
}

/// Counts the handover of the letters `intake` carries, so that a letter
/// whose receipt never comes is handed over at most [`TOOL_HANDOVERS`] times.
pub(crate) fn record_tool_handover(ledger: &mut Ledger, intake: &Intake, now: u64) -> bool {
    let mut changed = false;
    for letter in ledger
        .letters
        .iter_mut()
        .filter(|letter| intake.ids.contains(&letter.id))
    {
        letter.tool_handovers = letter.tool_handovers.saturating_add(1);
        letter.handed_at_unix_ms = Some(now);
        changed = true;
    }
    changed
}

/// Marks the pending letters `intake` hands over as handed: no bell is typed
/// for them again.
fn mark_handed(ledger: &mut Ledger, intake: &Intake, now: u64) {
    for letter in ledger
        .letters
        .iter_mut()
        .filter(|letter| letter.state == State::Pending && intake.ids.contains(&letter.id))
    {
        letter.handed = true;
        letter.handed_at_unix_ms = Some(now);
    }
}

/// The letter bodies for the turn Hide's bell opened.
#[cfg(test)]
pub(crate) fn pull(ledger: &Ledger, actor: &Actor) -> Result<Intake, String> {
    pull_in(
        ledger,
        actor,
        InterfaceLanguage::English,
        Bundling::PerSender,
    )
}

/// The bundle of every letter awaiting the caller's intake, its fold lines
/// worded in `language`.
fn pull_in(
    ledger: &Ledger,
    actor: &Actor,
    language: InterfaceLanguage,
    bundling: Bundling,
) -> Result<Intake, String> {
    actor.require_native_identity()?;
    if !prompt_hook(&actor.kind) {
        return Ok(Intake::default());
    }
    let pending: Vec<_> = ledger
        .letters
        .iter()
        .filter(|letter| letter.recipient.same_identity(actor) && letter.awaiting_intake())
        .collect();
    Ok(render(pending, language, bundling))
}

/// How a handover gives a caller its letters: as one bundle per sender, or,
/// for a `hide` older than bundles (a connected device whose kit is not yet
/// this build's), as it did before them: oldest first, none folded, at most
/// [`LEGACY_HOOK_LETTERS`], which is all such a kit's hook confirms, so its
/// letters keep flowing five at a time. Remove `Legacy` once no kit older
/// than this build can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Bundling {
    PerSender,
    Legacy,
}

/// The most letters a hook older than bundles confirms at once.
const LEGACY_HOOK_LETTERS: usize = 5;

impl Bundling {
    /// What the asking `hide` declared on its pull.
    pub(crate) fn of(command: &Command) -> Self {
        match command {
            Command::Pull { bundle: false, .. } => Self::Legacy,
            _ => Self::PerSender,
        }
    }

    fn limit(self) -> usize {
        match self {
            Self::PerSender => BUNDLE_LETTERS,
            Self::Legacy => LEGACY_HOOK_LETTERS,
        }
    }
}

/// Whether a bundle may fold `letter` into its sender's line: a question or
/// a block is always shown, since a folded one would go unanswered until it
/// expires and its sender would stay stuck.
fn foldable(letter: &Letter) -> bool {
    !matches!(letter.kind.as_str(), "request" | "block")
}

/// One sender's letters in a bundle: those shown in full, pending ones first
/// and each oldest first, and the older ones folded into one line.
struct Share<'a> {
    sender: &'a Actor,
    shown: Vec<&'a Letter>,
    folded: Vec<&'a Letter>,
}

/// `letters` as one bundle: grouped by sender in the order each sender's
/// first letter comes, every question and block shown, and of the rest the
/// newest [`SHOWN_PER_SENDER`] shown and the older ones folded.
fn bundle<'a>(letters: &[&'a Letter], bundling: Bundling) -> Vec<Share<'a>> {
    if bundling == Bundling::Legacy {
        return letters
            .iter()
            .map(|letter| Share {
                sender: &letter.sender,
                shown: vec![letter],
                folded: Vec::new(),
            })
            .collect();
    }
    let mut shares: Vec<Share<'a>> = Vec::new();
    for letter in letters {
        match shares
            .iter_mut()
            .find(|share| share.sender.same_identity(&letter.sender))
        {
            Some(share) => share.shown.push(letter),
            None => shares.push(Share {
                sender: &letter.sender,
                shown: vec![letter],
                folded: Vec::new(),
            }),
        }
    }
    for share in &mut shares {
        // Stable, so letters sent in the same millisecond keep the ledger's
        // order, which is the order they were sent.
        share.shown.sort_by_key(|letter| letter.created_at_unix_ms);
        let mut fold = share
            .shown
            .iter()
            .filter(|letter| foldable(letter))
            .count()
            .saturating_sub(SHOWN_PER_SENDER);
        let (folded, shown) = std::mem::take(&mut share.shown)
            .into_iter()
            .partition(|letter| {
                let folds = fold > 0 && foldable(letter);
                fold -= usize::from(folds);
                folds
            });
        share.folded = folded;
        share.shown = shown;
        // The letter a bell rings for comes first, so a backlog acknowledged
        // without a receipt never displaces it under the size cap.
        share
            .shown
            .sort_by_key(|letter| letter.state != State::Pending);
    }
    shares
}

/// How many folded ids a fold line names, newest first; `hide inbox` lists
/// every one.
const FOLD_IDS_NAMED: usize = 8;

/// The line that stands for a sender's folded letters: who sent them, how
/// many, in the interface language's words for the count, and how to open one.
fn fold_line(share: &Share<'_>, language: InterfaceLanguage) -> String {
    let words = super::bell::words(language);
    let mut named: Vec<&str> = share
        .folded
        .iter()
        .rev()
        .take(FOLD_IDS_NAMED)
        .map(|letter| letter.id.as_str())
        .collect();
    if share.folded.len() > FOLD_IDS_NAMED {
        named.push("…");
    }
    format!(
        "\nHide letters from {} ({}) {}: hide request show <id> ({})\n",
        share.sender.name,
        share.sender.kind,
        words.more(share.folded.len()),
        named.join(", ")
    )
}

/// The hook's context for `pending` as one bundle ([`bundle`]): each
/// sender's fold line, then its letters in full, within `HOOK_LIMIT` bytes,
/// with the ids to confirm, folded ones included. A sender whose fold line
/// no longer fits is left whole for the next handover, so a folded letter is
/// never shown in full later just because its newer ones were taken first.
fn render(mut pending: Vec<&Letter>, language: InterfaceLanguage, bundling: Bundling) -> Intake {
    // The letters a bell rings for come first, so a backlog acknowledged
    // without a receipt never displaces the one that opened this turn.
    pending.sort_by_key(|letter| (letter.state != State::Pending, letter.created_at_unix_ms));
    let mut context = String::new();
    let mut ids = Vec::new();
    // Reserve the tail before rendering bodies, including IDs and sender
    // metadata. Truncation always ends at a UTF-8 boundary.
    const TAIL_BUDGET: usize = 128;
    let limit = bundling.limit();
    'shares: for share in bundle(&pending, bundling) {
        if !share.folded.is_empty() {
            let line = fold_line(&share, language);
            if context.len() + line.len() + TAIL_BUDGET > HOOK_LIMIT
                || ids.len() + share.folded.len() > limit
            {
                break;
            }
            context.push_str(&line);
            ids.extend(share.folded.iter().map(|letter| letter.id.clone()));
        }
        for letter in share.shown {
            let header = format!(
                "\nHide letter {} from {} ({}) [{}]\n",
                letter.id, letter.sender.name, letter.sender.kind, letter.kind
            );
            let tail = format!("\nFull letter: hide request show {}\n", letter.id);
            let available =
                HOOK_LIMIT.saturating_sub(context.len() + header.len() + tail.len() + TAIL_BUDGET);
            if available == 0 || ids.len() == limit {
                break 'shares;
            }
            let mut end = letter.body.len().min(available);
            while !letter.body.is_char_boundary(end) {
                end -= 1;
            }
            context.push_str(&header);
            context.push_str(&letter.body[..end]);
            context.push_str(&tail);
            ids.push(letter.id.clone());
        }
    }
    let remaining = pending.len().saturating_sub(ids.len());
    if remaining > 0 {
        context.push_str(&format!(
            "\n대기 {remaining}통 더 있음. hide inbox로 확인하세요.\n"
        ));
    }
    Intake {
        context,
        ids,
        remaining,
    }
}

/// `hide inbox` for a `hide` older than bundles: every letter as before,
/// each pending one with its own `hide request ack`, which that `hide` can
/// run, since it has no `--ack`.
fn legacy_inbox(ledger: &Ledger, actor: &Actor) -> Value {
    let acknowledges = !prompt_hook(&actor.kind);
    Value::Array(
        ledger
            .letters
            .iter()
            .filter(|letter| {
                letter.recipient.same_identity(actor)
                    && matches!(letter.state, State::Pending | State::Undelivered)
                    || letter.sender.same_identity(actor) && letter.state == State::Undelivered
            })
            .map(|letter| {
                let mut entry = json!(letter);
                if acknowledges
                    && letter.state == State::Pending
                    && letter.recipient.same_identity(actor)
                {
                    entry["ack_command"] = json!(format!("hide request ack {}", letter.id));
                }
                entry
            })
            .collect(),
    )
}

/// What separates a bundle token's newest letter from its digest.
const BUNDLE_MARK: char = '~';

/// The token `hide inbox --ack` takes for the pending letters `ids`, in the
/// ledger's order: the newest's id, then the first 12 hex digits of the
/// SHA-256 of all of them, one per line, so the acknowledgement takes
/// exactly the letters the inbox listed and never one it did not.
fn bundle_token(ids: &[&str]) -> Option<String> {
    use sha2::{Digest, Sha256};
    let newest = ids.last()?;
    let digest = Sha256::digest(ids.join("\n").as_bytes());
    let short: String = digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Some(format!("{newest}{BUNDLE_MARK}{short}"))
}

/// `hide inbox`: the caller's letters as one bundle, as a handover gives
/// them, then its own letters that went undelivered; an agent with no prompt
/// hook acknowledges the whole bundle with the one command it carries.
fn inbox(ledger: &Ledger, actor: &Actor) -> Inbox {
    let mut received: Vec<_> = ledger
        .letters
        .iter()
        .filter(|letter| {
            letter.recipient.same_identity(actor)
                && matches!(letter.state, State::Pending | State::Undelivered)
        })
        .collect();
    received.sort_by_key(|letter| (letter.state != State::Pending, letter.created_at_unix_ms));
    let shares = bundle(&received, Bundling::PerSender);
    // The pending letters this answer lists, shown or folded, in the
    // ledger's order: the one acknowledgement takes exactly these.
    let listed: std::collections::HashSet<&str> = shares
        .iter()
        .flat_map(|share| share.shown.iter().chain(&share.folded))
        .filter(|letter| letter.state == State::Pending)
        .map(|letter| letter.id.as_str())
        .collect();
    let ids: Vec<&str> = ledger
        .letters
        .iter()
        .map(|letter| letter.id.as_str())
        .filter(|id| listed.contains(id))
        .collect();
    let ack_command = bundle_token(&ids)
        .filter(|_| !prompt_hook(&actor.kind))
        .map(|bundle| format!("hide inbox --ack {bundle}"));
    let folded = shares
        .iter()
        .filter(|share| !share.folded.is_empty())
        .map(|share| Folded {
            sender: share.sender.name.clone(),
            sender_kind: share.sender.kind.clone(),
            ids: share
                .folded
                .iter()
                .map(|letter| letter.id.clone())
                .collect(),
        })
        .collect();
    let letters = shares
        .into_iter()
        .flat_map(|share| share.shown)
        .cloned()
        .collect();
    let returned = ledger
        .letters
        .iter()
        .filter(|letter| {
            letter.sender.same_identity(actor)
                && !letter.recipient.same_identity(actor)
                && letter.state == State::Undelivered
        })
        .cloned()
        .collect();
    Inbox {
        letters,
        returned,
        folded,
        ack_command,
    }
}

#[cfg(test)]
mod tests {
    use super::super::RETENTION_MS;
    use super::super::ledger::AnswerWaitEnd;
    use super::*;
    fn actor(id: &str) -> Actor {
        Actor {
            pane_id: id.into(),
            name: id.into(),
            kind: "codex".into(),
            device_id: "local".into(),
            session: Some(format!("session-{id}")),
        }
    }

    #[test]
    fn pulled_letters_match_the_independent_delivery_envelope_contract() {
        let contract: serde_json::Value =
            serde_json::from_str(include_str!("../../../contracts/delivery-envelope.json"))
                .unwrap();
        let recipient = actor("recipient");
        for example in contract["examples"].as_array().unwrap() {
            let mut ledger = Ledger::default();
            let mut sender = actor("sender");
            sender.name = example["sender"].as_str().unwrap().into();
            sender.kind = example["sender_kind"].as_str().unwrap().into();
            // The operator's letter is made by the core's own identity.
            if example["operator"].as_bool() == Some(true) {
                sender = Actor::operator("local");
            }
            let letter = send(
                &mut ledger,
                &sender,
                &recipient,
                "contract",
                "Please look",
                example["kind"].as_str().unwrap(),
                None,
                1,
            )
            .unwrap();
            assert_eq!(letter.id, example["id"].as_str().unwrap());
            let intake = pull(&ledger, &recipient).unwrap();
            assert_eq!(
                intake.context.trim_start().lines().next(),
                example["first_line"].as_str(),
            );
            assert_eq!(intake.ids, [letter.id]);
        }

        let mut ledger = Ledger::default();
        for (index, example) in contract["batch"]["letters"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let mut sender = actor(&format!("sender-{index}"));
            sender.name = example["sender"].as_str().unwrap().into();
            sender.kind = example["sender_kind"].as_str().unwrap().into();
            send(
                &mut ledger,
                &sender,
                &recipient,
                "contract-batch",
                example["body"].as_str().unwrap(),
                example["kind"].as_str().unwrap(),
                None,
                index as u64 + 1,
            )
            .unwrap();
        }
        assert_eq!(
            pull(&ledger, &recipient).unwrap().context,
            contract["batch"]["context"].as_str().unwrap(),
        );
    }

    #[test]
    fn missing_native_identity_cannot_send_read_or_confirm_previous_occupant_mail() {
        let mut ledger = Ledger::default();
        let sender = actor("sender");
        let recipient = actor("recipient");
        let letter = send(
            &mut ledger,
            &sender,
            &recipient,
            "first",
            "private",
            "request",
            None,
            1,
        )
        .unwrap();
        let mut unknown = recipient.clone();
        unknown.session = None;
        assert_eq!(
            send(
                &mut ledger,
                &sender,
                &unknown,
                "second",
                "private",
                "request",
                None,
                2
            )
            .unwrap_err(),
            "native_identity_required"
        );
        // A retained legacy record with no native owner must not turn two
        // missing references in the same pane into authorization.
        ledger.letters[0].recipient.session = None;
        let before = ledger.clone();
        for command in [
            Command::Inbox { bundle: true },
            Command::Pull {
                prompt_digest: None,
                session: None,
                point: hide_node_link::panes::PullPoint::Prompt,
                bundle: true,
            },
            Command::Show {
                id: letter.id.clone(),
            },
            Command::Confirm {
                ids: vec![letter.id.clone()],
            },
            Command::Send {
                target: sender.pane_id.clone(),
                intent: "third".into(),
                body: "private".into(),
                kind: "request".into(),
                now: false,
                force: false,
            },
        ] {
            assert_eq!(
                apply(&mut ledger, &unknown, Some(&sender), &command, 2).unwrap_err(),
                "native_identity_required"
            );
            assert_eq!(ledger, before);
        }
        assert_eq!(
            pull(&ledger, &unknown).unwrap_err(),
            "native_identity_required"
        );
        let mut replacement = recipient.clone();
        replacement.session = Some("new-occupant".into());
        assert!(pull(&ledger, &replacement).unwrap().ids.is_empty());
        for command in [
            Command::Show {
                id: letter.id.clone(),
            },
            Command::Confirm {
                ids: vec![letter.id],
            },
        ] {
            assert_eq!(
                apply(&mut ledger, &replacement, None, &command, 2).unwrap_err(),
                "letter_unavailable"
            );
            assert_eq!(ledger, before);
        }
    }
    #[test]
    fn same_intent_converges_across_confirmation_cancel_and_restart() {
        let mut ledger = Ledger::default();
        let a = actor("a");
        let b = actor("b");
        let letter = send(&mut ledger, &a, &b, "intent", "body", "request", None, 1).unwrap();
        assert_eq!(pull(&ledger, &b).unwrap().ids, vec![letter.id.clone()]);
        assert_eq!(pull(&ledger, &b).unwrap().ids, vec![letter.id.clone()]);
        apply(
            &mut ledger,
            &b,
            None,
            &Command::Confirm {
                ids: vec![letter.id.clone()],
            },
            2,
        )
        .unwrap();
        assert!(pull(&ledger, &b).unwrap().ids.is_empty());
        apply(
            &mut ledger,
            &a,
            None,
            &Command::Cancel {
                id: letter.id.clone(),
            },
            3,
        )
        .unwrap();
        let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
        assert_eq!(
            send(&mut restored, &a, &b, "intent", "body", "request", None, 4)
                .unwrap()
                .id,
            letter.id
        );
        assert_eq!(restored.letters.len(), 1);
        assert_eq!(restored.letters[0].state, State::Cancelled);
    }
    #[test]
    fn intake_shows_every_question_and_is_bounded_utf8_without_losing_full_body() {
        let mut ledger = Ledger::default();
        let a = actor("a");
        let b = actor("b");
        for index in 0..7 {
            send(
                &mut ledger,
                &a,
                &b,
                &format!("i{index}"),
                "body",
                "request",
                None,
                index,
            )
            .unwrap();
        }
        let intake = pull(&ledger, &b).unwrap();
        assert_eq!(intake.ids.len(), 7, "a question is never folded");
        assert_eq!(intake.remaining, 0);
        ledger.letters[0].body = "한".repeat(BODY_LIMIT / 3);
        let intake = pull(&ledger, &b).unwrap();
        assert!(intake.context.len() <= HOOK_LIMIT);
        assert_eq!(intake.ids, vec![ledger.letters[0].id.clone()]);
        assert!(intake.context.contains("대기 6통 더 있음"));
        assert!(intake.context.contains("hide request show letter-1"));
        assert_eq!(ledger.letters[0].body.len(), (BODY_LIMIT / 3) * 3);
    }
    #[test]
    fn expiry_and_capacity_are_explicit_without_eviction() {
        let mut ledger = Ledger::default();
        let a = actor("a");
        let b = actor("b");
        let letter = send(&mut ledger, &a, &b, "i", "body", "request", None, 10).unwrap();
        assert!(!ledger.expire(10 + super::super::DELIVERY_EXPIRY_MS - 1));
        assert!(ledger.expire(10 + super::super::DELIVERY_EXPIRY_MS));
        assert_eq!(ledger.letters[0].state, State::Undelivered);
        assert_eq!(
            send(
                &mut ledger,
                &a,
                &b,
                "big",
                &"x".repeat(BODY_LIMIT + 1),
                "request",
                None,
                20
            )
            .unwrap_err(),
            "capacity"
        );
        assert_eq!(ledger.letters[0].id, letter.id);
        // Its sender gets it back apart from the letters it received.
        let inbox = apply(&mut ledger, &a, None, &Command::Inbox { bundle: true }, 20).unwrap();
        assert_eq!(inbox["letters"], json!([]));
        assert_eq!(inbox["returned"][0]["id"], letter.id.as_str());
        assert_eq!(inbox["returned"][0]["state"], "undelivered");
        assert_eq!(inbox["returned"][0]["recipient"]["pane_id"], "b");
    }

    #[test]
    fn repeated_intent_does_not_require_a_live_target_or_close_another_request() {
        let mut ledger = Ledger::default();
        let a = actor("a");
        let b = actor("b");
        let first = send(&mut ledger, &a, &b, "first", "work", "request", None, 1).unwrap();
        let retry = apply(
            &mut ledger,
            &a,
            None,
            &Command::Send {
                target: "gone".into(),
                intent: "first".into(),
                body: "work".into(),
                kind: "request".into(),
                now: false,
                force: false,
            },
            2,
        )
        .unwrap();
        assert_eq!(retry["id"], first.id);
        let second = send(&mut ledger, &a, &b, "second", "other", "request", None, 3).unwrap();
        let reply = apply(
            &mut ledger,
            &b,
            None,
            &Command::Reply {
                id: first.id,
                intent: "reply".into(),
                body: "done".into(),
            },
            4,
        )
        .unwrap();
        let retry = apply(
            &mut ledger,
            &b,
            None,
            &Command::Reply {
                id: second.id.clone(),
                intent: "reply".into(),
                body: "different decision".into(),
            },
            5,
        )
        .unwrap();
        assert_eq!(retry["id"], reply["id"]);
        assert!(
            ledger
                .letters
                .iter()
                .find(|letter| letter.id == second.id)
                .unwrap()
                .waiting_answer
        );
        assert_eq!(ledger.letters.len(), 3);
    }

    fn send_command(intent: &str, kind: &str, body: &str) -> Command {
        Command::Send {
            target: "b".into(),
            intent: intent.into(),
            body: body.into(),
            kind: kind.into(),
            now: false,
            force: false,
        }
    }

    #[test]
    fn an_intent_reused_for_another_recipient_is_refused_not_answered_with_the_first_letter() {
        let mut ledger = Ledger::default();
        let (a, b, c) = (actor("a"), actor("b"), actor("c"));
        let first = send(&mut ledger, &a, &b, "once", "1/2", "progress", None, 1).unwrap();
        // The same letter again is a retry.
        let retry = send(&mut ledger, &a, &b, "once", "1/2", "progress", None, 2).unwrap();
        assert_eq!(retry.id, first.id);
        // Another recipient is not, whether it arrives as a target or as a key.
        let elsewhere = send(&mut ledger, &a, &c, "once", "1/2", "progress", None, 3);
        assert_eq!(elsewhere, Err("intent_conflict".into()));
        let command = send_command("once", "progress", "1/2");
        let applied = apply(&mut ledger, &a, Some(&c), &command, 4);
        assert_eq!(applied, Err("intent_conflict".into()));
        assert_eq!(ledger.letters.len(), 1);
    }

    #[test]
    fn an_intent_reused_with_another_kind_or_body_is_refused() {
        let mut ledger = Ledger::default();
        let (a, b) = (actor("a"), actor("b"));
        send(&mut ledger, &a, &b, "once", "1/2", "progress", None, 1).unwrap();
        for (kind, body) in [("progress", "2/2"), ("request", "1/2")] {
            let command = send_command("once", kind, body);
            assert_eq!(
                apply(&mut ledger, &a, None, &command, 2),
                Err("intent_conflict".into()),
                "{kind} {body}"
            );
        }
        assert_eq!(ledger.letters.len(), 1);
    }

    #[test]
    fn a_report_retry_matches_the_stored_report_that_gained_a_views_list() {
        let mut ledger = Ledger::default();
        let (a, b) = (actor("a"), actor("b"));
        let listed = "done\n\nViews this agent opened (read one):\n- v1 page (seen)";
        let first = send(&mut ledger, &a, &b, "report", listed, "report", None, 1).unwrap();
        let retry = apply(
            &mut ledger,
            &a,
            None,
            &send_command("report", "report", "done"),
            2,
        )
        .unwrap();
        assert_eq!(retry["id"], first.id.as_str());
        let other = apply(
            &mut ledger,
            &a,
            None,
            &send_command("report", "report", "something else"),
            3,
        );
        assert_eq!(other, Err("intent_conflict".into()));
    }

    fn pending_for(ledger: &mut Ledger, recipient: &Actor, count: usize) -> Vec<String> {
        (0..count)
            .map(|index| {
                send(
                    ledger,
                    &actor(&format!("sender-{index}")),
                    recipient,
                    &format!("intent-{}-{index}", recipient.pane_id),
                    &format!("body-{index}"),
                    "request",
                    None,
                    index as u64 + 1,
                )
                .unwrap()
                .id
            })
            .collect()
    }

    /// The line a test rings with, standing for what the doorbell typed.
    const LINE: &str = "🦉 sender 보고: body-0";

    /// What the prompt hook receives when `prompt` was submitted.
    fn pull_for(ledger: &mut Ledger, recipient: &Actor, prompt: &str) -> Intake {
        let answer = apply(
            ledger,
            recipient,
            None,
            &Command::Pull {
                prompt_digest: Some(hide_agent_hooks::delivery::prompt_digest(prompt)),
                session: None,
                point: hide_node_link::panes::PullPoint::Prompt,
                bundle: true,
            },
            100,
        )
        .unwrap();
        serde_json::from_value(answer).unwrap()
    }

    /// The doorbell typing `line` for the letter at `index`.
    fn ring(ledger: &mut Ledger, index: usize, line: &str) {
        ledger.letters[index]
            .reserve_bell(&Default::default(), 50, line.into())
            .unwrap();
    }

    #[test]
    fn an_observe_only_bell_turn_takes_its_batch_at_either_gap_and_only_once_confirmed() {
        for point in [PullPoint::Tool, PullPoint::TurnEnd] {
            let mut ledger = Ledger::default();
            let mut recipient = actor("recipient");
            recipient.kind = "grok".into();
            recipient.session = crate::engine::tokens::session_digest("native-session");
            let ids = pending_for(&mut ledger, &recipient, 2);
            // A receipt-less legacy acknowledgement follows the new letter,
            // just as it does at a context-carrying prompt hook.
            ledger.letters[1].state = State::Acknowledged;
            ledger.letters[1].finished_at_unix_ms = Some(1);
            ring(&mut ledger, 0, LINE);
            let prompt = |text: &str, session: &str| Command::Pull {
                prompt_digest: Some(hide_agent_hooks::delivery::prompt_digest(text)),
                session: Some(session.into()),
                point: PullPoint::Prompt,
                bundle: true,
            };
            assert_eq!(
                apply(
                    &mut ledger,
                    &recipient,
                    None,
                    &prompt(LINE, "another-session"),
                    100
                ),
                Err("session_mismatch".into())
            );
            let answer = apply(
                &mut ledger,
                &recipient,
                None,
                // A console reader can omit the owl before the prompt hook.
                &prompt("sender 보고: body-0", "native-session"),
                100,
            )
            .unwrap();
            assert_eq!(answer, json!({"context":"", "ids":[], "remaining":0}));
            // A letter arriving after the bell submission cannot join its batch.
            let later = send(
                &mut ledger,
                &actor("late"),
                &recipient,
                "late",
                "late-body",
                "request",
                None,
                101,
            )
            .unwrap();
            apply(
                &mut ledger,
                &recipient,
                None,
                &prompt("sender 보고: body-0", "native-session"),
                102,
            )
            .unwrap();
            let intake = gap_pull(
                &ledger,
                &recipient,
                point,
                Some(100),
                102,
                InterfaceLanguage::English,
                Bundling::PerSender,
            )
            .unwrap();
            assert_eq!(intake.ids, ids);
            assert!(!intake.context.contains("late-body"));
            record_tool_handover(&mut ledger, &intake, 1001);
            apply(
                &mut ledger,
                &recipient,
                None,
                &Command::Confirm { ids },
                103,
            )
            .unwrap();
            assert!(
                gap_pull(
                    &ledger,
                    &recipient,
                    point,
                    Some(100),
                    104,
                    InterfaceLanguage::English,
                    Bundling::PerSender
                )
                .unwrap()
                .ids
                .is_empty()
            );
            // A due or explicit immediate letter still travels at either gap.
            ledger
                .letters
                .iter_mut()
                .find(|letter| letter.id == later.id)
                .unwrap()
                .immediate = true;
            assert_eq!(
                gap_pull(
                    &ledger,
                    &recipient,
                    point,
                    Some(100),
                    105,
                    InterfaceLanguage::English,
                    Bundling::PerSender
                )
                .unwrap()
                .ids,
                [later.id]
            );
        }
    }

    #[test]
    fn an_operator_submission_releases_the_previous_bell_turn_claim() {
        let mut ledger = Ledger::default();
        let mut recipient = actor("recipient");
        recipient.kind = "grok".into();
        recipient.session = crate::engine::tokens::session_digest("native-session");
        pending_for(&mut ledger, &recipient, 1);
        ring(&mut ledger, 0, LINE);
        for text in [LINE, "the operator's own prompt"] {
            let answer = apply(
                &mut ledger,
                &recipient,
                None,
                &Command::Pull {
                    prompt_digest: Some(hide_agent_hooks::delivery::prompt_digest(text)),
                    session: Some("native-session".into()),
                    point: PullPoint::Prompt,
                    bundle: true,
                },
                100,
            )
            .unwrap();
            assert_eq!(answer["context"], "");
            assert_eq!(
                gap_pull(
                    &ledger,
                    &recipient,
                    PullPoint::TurnEnd,
                    Some(100),
                    101,
                    InterfaceLanguage::English,
                    Bundling::PerSender
                )
                .unwrap()
                .ids
                .len(),
                usize::from(text == LINE)
            );
        }
        for point in [PullPoint::Tool, PullPoint::TurnEnd] {
            assert!(
                gap_pull(
                    &ledger,
                    &recipient,
                    point,
                    Some(100),
                    101,
                    InterfaceLanguage::English,
                    Bundling::PerSender
                )
                .unwrap()
                .ids
                .is_empty()
            );
        }
    }

    #[test]
    fn a_transport_without_the_owl_opens_only_its_recipients_pending_letter() {
        let mut ledger = Ledger::default();
        let recipient = actor("recipient");
        let other = actor("other");
        let ids = pending_for(&mut ledger, &recipient, 1);
        send(
            &mut ledger,
            &actor("sender-other"),
            &other,
            "other",
            "body-0",
            "request",
            None,
            2,
        )
        .unwrap();
        ring(&mut ledger, 0, "🦉 sender 보고: body-0");
        let prompt = "sender 보고: body-0";
        assert_eq!(pull_for(&mut ledger, &recipient, prompt).ids, ids);
        for changed in [
            "sender 보고: body-1",
            "sender 보고: body-0!",
            "sender 보고: body-0\u{fe0f}",
        ] {
            assert!(pull_for(&mut ledger, &recipient, changed).ids.is_empty());
        }
        assert!(pull_for(&mut ledger, &other, prompt).ids.is_empty());
        for state in [State::Delivered, State::Cancelled, State::Undelivered] {
            ledger.letters[0].state = state;
            assert!(pull_for(&mut ledger, &recipient, prompt).ids.is_empty());
        }
    }

    #[test]
    fn old_and_canonical_hook_digests_share_the_saved_bell_during_an_upgrade() {
        let mut ledger = Ledger::default();
        let recipient = actor("recipient");
        let ids = pending_for(&mut ledger, &recipient, 1);
        // Persist and reload a pre-upgrade reservation, with no migration.
        ring(&mut ledger, 0, "🔔 sender 보고: body-0");
        let mut ledger: Ledger =
            serde_json::from_slice(&serde_json::to_vec(&ledger).unwrap()).unwrap();
        // SHA-256 values from Python hashlib, not from the shared function.
        for digest in [
            "905ad85a944e8f21f9baf7bf9f8c015ee474ce4aa06aa3ceb4819983910ad7cc",
            "4f92fb268cec536579de240550e4f59959e98f09adcce532f7c6df805b089883",
        ] {
            let answer = apply(
                &mut ledger,
                &recipient,
                None,
                &Command::Pull {
                    prompt_digest: Some(digest.into()),
                    session: None,
                    point: PullPoint::Prompt,
                    bundle: true,
                },
                100,
            )
            .unwrap();
            let intake: Intake = serde_json::from_value(answer).unwrap();
            assert_eq!(intake.ids, ids, "{digest}");
        }
    }

    #[test]
    fn an_operator_prompt_gets_a_count_while_a_bell_is_coming_and_the_bell_gets_the_bodies() {
        let mut ledger = Ledger::default();
        let recipient = actor("recipient");
        let ids = pending_for(&mut ledger, &recipient, 2);
        ring(&mut ledger, 0, LINE);
        let operator = pull_for(&mut ledger, &recipient, "please run the tests");
        assert!(operator.ids.is_empty(), "nothing to confirm");
        assert_eq!(
            operator.context.trim(),
            "Hide 편지 2통 대기 중, 이 턴이 끝난 뒤 전달"
        );
        assert!(!operator.context.contains("body-"));
        let bell = pull_for(&mut ledger, &recipient, &format!("{LINE}\n"));
        assert_eq!(bell.ids, ids);
        assert!(bell.context.contains("body-0") && bell.context.contains("body-1"));
        assert!(!bell.context.contains("대기 중"));
        // Neither pull confirms anything.
        assert_eq!(
            ledger
                .letters
                .iter()
                .filter(|l| l.intake_confirmed())
                .count(),
            0
        );
    }

    #[test]
    fn only_the_line_the_doorbell_typed_for_the_callers_pending_letter_is_the_bell() {
        let mut ledger = Ledger::default();
        let recipient = actor("recipient");
        let other = actor("other");
        pending_for(&mut ledger, &recipient, 1);
        send(
            &mut ledger,
            &actor("sender-0"),
            &other,
            "intent-other",
            "body-0",
            "request",
            None,
            2,
        )
        .unwrap();
        let counted = |intake: Intake| intake.ids.is_empty() && intake.context.contains("대기 중");
        // A person typing what a bell would say, before any bell rang.
        assert!(counted(pull_for(&mut ledger, &recipient, LINE)));
        ring(&mut ledger, 0, LINE);
        assert!(counted(pull_for(
            &mut ledger,
            &recipient,
            &format!("{LINE} 그리고")
        )));
        assert!(counted(pull_for(
            &mut ledger,
            &recipient,
            "🦉 sender 보고: body-1"
        )));
        // The same words in another agent's pane are not its bell.
        assert!(counted(pull_for(&mut ledger, &other, LINE)));
        // A second bell for the letter replaces the line the first typed.
        let second = "🦉 sender 보고: body-0 · 외 1통";
        ring(&mut ledger, 0, second);
        assert!(counted(pull_for(&mut ledger, &recipient, LINE)));
        let bell = pull_for(&mut ledger, &recipient, second);
        assert_eq!(bell.ids, [ledger.letters[0].id.clone()]);
        // The bell's pull handed it over: no bell is typed for it again.
        assert!(ledger.letters[0].handed);
        assert_eq!(
            ledger.letters[0].reserve_bell(&Default::default(), 60, LINE.into()),
            Err("letter_handed".into())
        );
        // Once the letter is taken in, its line opens nothing.
        let id = ledger.letters[0].id.clone();
        apply(
            &mut ledger,
            &recipient,
            None,
            &Command::Confirm { ids: vec![id] },
            100,
        )
        .unwrap();
        let after = pull_for(&mut ledger, &recipient, "🦉 sender 보고: body-0 · 외 1통");
        assert!(after.ids.is_empty() && after.context.is_empty());
    }

    /// A letter the bell's pull handed over but no confirmation took in
    /// rides the next bell's batch for the same recipient, is not counted as
    /// a bell still coming, and at its deadline ends delivered without a
    /// receipt, never undelivered; a letter never handed ends undelivered.
    #[test]
    fn a_handed_letter_rides_the_next_bell_and_never_ends_undelivered() {
        let mut ledger = Ledger::default();
        let recipient = actor("recipient");
        let first = pending_for(&mut ledger, &recipient, 1).remove(0);
        ring(&mut ledger, 0, LINE);
        assert_eq!(
            pull_for(&mut ledger, &recipient, LINE).ids,
            [first.as_str()]
        );
        let next = send(
            &mut ledger,
            &actor("sender"),
            &recipient,
            "next",
            "body-next",
            "request",
            None,
            60,
        )
        .unwrap()
        .id;
        let counted = pull_for(&mut ledger, &recipient, "the operator's own prompt");
        assert!(counted.ids.is_empty());
        assert!(counted.context.contains("1통"), "{}", counted.context);
        let line = "🦉 sender 보고: body-next";
        ring(&mut ledger, 1, line);
        assert_eq!(
            pull_for(&mut ledger, &recipient, line).ids,
            [first.clone(), next.clone()]
        );
        let unrung = send(
            &mut ledger,
            &actor("sender"),
            &actor("other-recipient"),
            "unrung",
            "body-unrung",
            "request",
            None,
            61,
        )
        .unwrap()
        .id;
        assert!(ledger.expire(super::super::DELIVERY_EXPIRY_MS + 1_000));
        let state = |ledger: &Ledger, id: &str| {
            let letter = ledger
                .letters
                .iter()
                .find(|letter| letter.id == id)
                .unwrap();
            (
                letter.state,
                letter.hook_confirmed,
                letter.intake_confirmed(),
            )
        };
        assert_eq!(state(&ledger, &first), (State::Delivered, None, false));
        assert_eq!(state(&ledger, &next), (State::Delivered, None, false));
        assert_eq!(state(&ledger, &unrung).0, State::Undelivered);
        // A confirmation that comes late still takes it in.
        apply(
            &mut ledger,
            &recipient,
            None,
            &Command::Confirm {
                ids: vec![first.clone()],
            },
            super::super::DELIVERY_EXPIRY_MS + 2_000,
        )
        .unwrap();
        assert_eq!(state(&ledger, &first), (State::Delivered, Some(true), true));
    }

    /// A handed letter whose recipient's wake failed is undelivered, as any
    /// letter there is: the wake's failure, not a missing receipt, ends it.
    #[test]
    fn a_handed_letter_whose_wake_failed_is_undelivered() {
        let mut ledger = Ledger::default();
        let recipient = actor("recipient");
        let id = pending_for(&mut ledger, &recipient, 1).remove(0);
        ring(&mut ledger, 0, LINE);
        pull_for(&mut ledger, &recipient, LINE);
        assert!(ledger.letters[0].handed);
        let failed = std::collections::BTreeMap::from([(id, "wake_failed".to_owned())]);
        assert!(ledger.fail_undeliverable(&failed, 100));
        assert_eq!(ledger.letters[0].state, State::Undelivered);
    }

    /// What a pane's node hands over confirms only the letters to its
    /// recipient's own session that still await intake, as the hook's
    /// confirmation would: a report ends its watch at once rather than at
    /// its deadline, and another session's record confirms nothing.
    #[test]
    fn owed_confirmations_take_in_only_the_recipients_own_sessions_letters() {
        let mut ledger = Ledger::default();
        let mut parent = actor("parent");
        parent.session = crate::engine::tokens::session_digest("parent-native");
        let child = actor("child");
        super::super::watch::start(&mut ledger, &parent, &child, 0).unwrap();
        let report = send(
            &mut ledger,
            &child,
            &parent,
            "done",
            "body-0",
            "report",
            None,
            1,
        )
        .unwrap()
        .id;
        let to_child = send(
            &mut ledger,
            &parent,
            &child,
            "ask",
            "body-1",
            "request",
            None,
            2,
        )
        .unwrap()
        .id;
        let line = "🦉 child 보고: body-0";
        ring(&mut ledger, 0, line);
        assert_eq!(pull_for(&mut ledger, &parent, line).ids, [report.as_str()]);
        let owed = |session: &str| hide_agent_hooks::owed::Owed {
            session: session.to_owned(),
            ids: vec![report.clone(), to_child.clone()],
        };
        assert!(confirm_owed(&mut ledger, &parent, &owed("other-native"), 3).is_empty());
        assert_eq!(ledger.letters[0].state, State::Pending);
        assert_eq!(
            confirm_owed(&mut ledger, &parent, &owed("parent-native"), 4),
            std::slice::from_ref(&report)
        );
        assert_eq!(
            (ledger.letters[0].state, ledger.letters[0].hook_confirmed),
            (State::Delivered, Some(true))
        );
        assert!(ledger.watches.is_empty(), "the report ended its watch");
        assert_eq!(ledger.letters[1].state, State::Pending, "not the parent's");
        assert!(confirm_owed(&mut ledger, &parent, &owed("parent-native"), 5).is_empty());
    }

    /// A report a prompt hook handed over ends its parent's watch at its
    /// deadline, as a confirmation would; one no hook handed ends
    /// undelivered and leaves the watch.
    #[test]
    fn a_handed_report_ends_its_watch_at_the_deadline_and_an_unhanded_one_does_not() {
        for handed in [true, false] {
            let mut ledger = Ledger::default();
            let parent = actor("parent");
            let child = actor("child");
            super::super::watch::start(&mut ledger, &parent, &child, 0).unwrap();
            let report = send(
                &mut ledger,
                &child,
                &parent,
                "done",
                "body-0",
                "report",
                None,
                1,
            )
            .unwrap()
            .id;
            let line = "🦉 child 보고: body-0";
            ring(&mut ledger, 0, line);
            if handed {
                assert_eq!(pull_for(&mut ledger, &parent, line).ids, [report.as_str()]);
            }
            assert!(ledger.expire(super::super::DELIVERY_EXPIRY_MS + 1_000));
            let state = ledger.letters[0].state;
            assert_eq!(
                (state, ledger.watches.is_empty()),
                if handed {
                    (State::Delivered, true)
                } else {
                    (State::Undelivered, false)
                }
            );
        }
    }

    #[test]
    fn the_operators_prompt_never_carries_a_body_and_counts_only_letters_a_bell_will_bring() {
        let mut ledger = Ledger::default();
        let recipient = actor("recipient");
        pending_for(&mut ledger, &recipient, 3);
        // Three bells spent: the letter waits for `hide inbox` and expires.
        ledger.letters[0].bell_attempts = Some(3);
        // Acknowledged before intake: no bell either.
        ledger.letters[1].state = State::Acknowledged;
        ledger.letters[1].hook_confirmed = Some(false);
        let operator = pull_for(&mut ledger, &recipient, "operator");
        assert!(operator.ids.is_empty());
        assert_eq!(
            operator.context.trim(),
            "Hide 편지 1통 대기 중, 이 턴이 끝난 뒤 전달"
        );
        ledger.letters[2].bell_attempts = Some(3);
        let operator = pull_for(&mut ledger, &recipient, "operator");
        assert!(operator.ids.is_empty() && operator.context.is_empty());
    }

    #[test]
    fn a_prompt_hook_that_runs_inside_an_agent_without_one_hands_over_nothing() {
        for kind in ["gemini", "cursor"] {
            let mut ledger = Ledger::default();
            let mut recipient = actor("recipient");
            recipient.kind = kind.into();
            pending_for(&mut ledger, &recipient, 1);
            ring(&mut ledger, 0, LINE);
            for prompt in ["operator", LINE] {
                let intake = pull_for(&mut ledger, &recipient, prompt);
                assert!(intake.ids.is_empty() && intake.context.is_empty(), "{kind}");
            }
        }
    }

    /// PRD letters-to-busy-agents D-02, D-04, D-05, D-10, D-11: at a tool
    /// event an agent of a kind whose hook takes letters there gets, oldest
    /// first, the pending letters whose wait is over, counted from the later
    /// of arrival and the start of its work, and a `--now` letter at once; a
    /// resting agent, a letter still waiting, a letter past its deadline and
    /// a kind without a tool-letter hook get nothing.
    #[test]
    fn a_tool_event_hands_a_working_agent_letters_only_when_its_adapter_takes_them() {
        let wait = super::super::doorbell::MID_TURN_WAIT_MS;
        for kind in ["opencode", "pi", "omp", "grok", "antigravity"] {
            assert!(tool_letters(kind), "{kind}");
            let mut ledger = Ledger::default();
            let mut recipient = actor("recipient");
            recipient.kind = kind.into();
            // Arrived at 1 and 2.
            let ids = pending_for(&mut ledger, &recipient, 2);
            let pull = |ledger: &Ledger, since, now| {
                tool_pull(ledger, &recipient, since, now).unwrap().ids
            };
            assert!(pull(&ledger, None, 2 + wait).is_empty(), "{kind}: resting");
            assert!(pull(&ledger, Some(1), wait).is_empty(), "{kind}: waiting");
            assert_eq!(pull(&ledger, Some(1), 1 + wait), ids[..1], "{kind}");
            assert_eq!(pull(&ledger, Some(1), 2 + wait), ids, "{kind}");
            // Work that began at 1000 counts from then.
            assert!(pull(&ledger, Some(1000), 2 + wait).is_empty(), "{kind}");
            assert_eq!(pull(&ledger, Some(1000), 1000 + wait), ids, "{kind}");
            // A --now letter at the next tool event.
            ledger.letters[1].immediate = true;
            assert_eq!(pull(&ledger, Some(1000), 1001), ids[1..], "{kind}");
            let intake = tool_pull(&ledger, &recipient, Some(1000), 1001).unwrap();
            assert!(intake.context.contains("body-1") && !intake.context.contains("body-0"));
            // Taken up like a prompt's: confirmed, not received, until then.
            assert!(
                ledger
                    .letters
                    .iter()
                    .all(|letter| letter.state == State::Pending)
            );
            // Handed over three times without a receipt, a letter waits for
            // the next prompt; its batch-mate still goes.
            for _ in 0..TOOL_HANDOVERS {
                let intake = tool_pull(&ledger, &recipient, Some(1000), 1001).unwrap();
                assert!(record_tool_handover(&mut ledger, &intake, 1001), "{kind}");
            }
            assert_eq!(ledger.letters[1].tool_handovers, TOOL_HANDOVERS);
            assert!(pull(&ledger, Some(1000), 1001).is_empty(), "{kind}: spent");
            assert_eq!(pull(&ledger, Some(1000), 1000 + wait), ids[..1], "{kind}");
            let none = tool_pull(&ledger, &recipient, Some(1000), 1001).unwrap();
            assert!(!record_tool_handover(&mut ledger, &none, 1001), "{kind}");
            // A letter past its deadline is not handed over.
            ledger.letters[1].created_at_unix_ms = 0;
            assert!(pull(&ledger, Some(1000), 61 * 60_000).is_empty(), "{kind}");
        }
        for kind in ["claude", "codex", "gemini", "cursor", "future-agent"] {
            assert!(!tool_letters(kind), "{kind}");
            let mut ledger = Ledger::default();
            let mut recipient = actor("recipient");
            recipient.kind = kind.into();
            pending_for(&mut ledger, &recipient, 1);
            ledger.letters[0].immediate = true;
            assert!(
                tool_pull(&ledger, &recipient, Some(0), 2 + wait)
                    .unwrap()
                    .ids
                    .is_empty(),
                "{kind}"
            );
        }
        // Through the command table the ask carries no pane, so it is empty:
        // the worker answers it with the pane's own observation.
        let mut ledger = Ledger::default();
        let mut recipient = actor("recipient");
        recipient.kind = "opencode".into();
        pending_for(&mut ledger, &recipient, 1);
        ledger.letters[0].immediate = true;
        let answer = apply(
            &mut ledger,
            &recipient,
            None,
            &Command::Pull {
                prompt_digest: None,
                session: None,
                point: hide_node_link::panes::PullPoint::Tool,
                bundle: true,
            },
            2 + wait,
        )
        .unwrap();
        assert_eq!(answer, json!(Intake::default()));
    }

    #[test]
    fn an_agent_no_bell_rings_for_gets_its_letters_on_any_prompt_and_confirms_them_later() {
        let mut ledger = Ledger::default();
        let mut recipient = actor("recipient");
        recipient.kind = "opencode".into();
        let ids = pending_for(&mut ledger, &recipient, 2);
        for prompt in ["operator", LINE] {
            let intake = pull_for(&mut ledger, &recipient, prompt);
            assert_eq!(intake.ids, ids, "{prompt}");
            assert!(intake.context.contains("body-0") && intake.context.contains("body-1"));
            assert!(!intake.context.contains("대기 중"));
        }
        // A pull is not a receipt: the plugin confirms after OpenCode stored
        // the prompt, and a prompt lost before that leaves them pending.
        assert!(
            ledger
                .letters
                .iter()
                .all(|letter| !letter.intake_confirmed())
        );
        apply(
            &mut ledger,
            &recipient,
            None,
            &Command::Confirm { ids: ids.clone() },
            101,
        )
        .unwrap();
        assert!(ledger.letters.iter().all(Letter::intake_confirmed));
        assert!(pull_for(&mut ledger, &recipient, LINE).ids.is_empty());
    }

    #[test]
    fn acknowledging_is_the_receipt_for_every_agent_kind() {
        for kind in ["grok", "gemini", "codex", "claude"] {
            let mut ledger = Ledger::default();
            let child = actor("child");
            let mut parent = actor("parent");
            parent.kind = kind.into();
            super::super::watch::start(&mut ledger, &parent, &child, 1).unwrap();
            let report = send(
                &mut ledger,
                &child,
                &parent,
                "report",
                "done",
                "report",
                None,
                2,
            )
            .unwrap();
            let acknowledged = apply(
                &mut ledger,
                &parent,
                None,
                &Command::Ack { id: report.id },
                3,
            )
            .unwrap();
            assert_eq!(acknowledged["state"], "acknowledged", "{kind}");
            assert_eq!(acknowledged["hook_confirmed"], true, "{kind}");
            assert!(!ledger.letters[0].open(), "{kind}");
            assert!(ledger.watches.is_empty(), "{kind}");
            assert!(pull(&ledger, &parent).unwrap().ids.is_empty(), "{kind}");
        }
    }

    #[test]
    fn a_pull_from_an_older_kit_is_an_operator_prompt_pull() {
        for (wire, session) in [
            (json!({"op":"pull"}), None),
            (
                json!({"op":"pull","bell":true,"session":"s"}),
                Some("s".into()),
            ),
            (json!({"op":"pull","bell":false}), None),
        ] {
            assert_eq!(
                Command::from_wire(wire).unwrap(),
                Command::Pull {
                    prompt_digest: None,
                    session,
                    point: hide_node_link::panes::PullPoint::Prompt,
                    bundle: false,
                }
            );
        }
    }

    #[test]
    fn the_inbox_shows_an_agent_without_a_prompt_hook_the_command_that_acknowledges() {
        let mut ledger = Ledger::default();
        let mut recipient = actor("recipient");
        recipient.kind = "grok".into();
        let ids = pending_for(&mut ledger, &recipient, 1);
        let inbox = apply(
            &mut ledger,
            &recipient,
            None,
            &Command::Inbox { bundle: true },
            5,
        )
        .unwrap();
        assert_eq!(
            inbox["ack_command"],
            format!(
                "hide inbox --ack {}",
                bundle_token(&[ids[0].as_str()]).unwrap()
            )
        );
        // An expired letter can no longer be acknowledged, so none is offered.
        assert!(ledger.expire(10 + super::super::DELIVERY_EXPIRY_MS));
        let inbox = apply(
            &mut ledger,
            &recipient,
            None,
            &Command::Inbox { bundle: true },
            5,
        )
        .unwrap();
        assert_eq!(inbox["letters"][0]["state"], "undelivered");
        assert!(inbox.get("ack_command").is_none());
        // A hooked agent's prompt hook acknowledges for it; the sender sees none.
        let hooked = actor("hooked");
        pending_for(&mut ledger, &hooked, 1);
        let inbox = apply(
            &mut ledger,
            &hooked,
            None,
            &Command::Inbox { bundle: true },
            5,
        )
        .unwrap();
        assert!(inbox.get("ack_command").is_none());
    }

    /// docs/delivery.md, Watch activity and completion: an interim report
    /// such as a plan is a `progress` letter, delivered like a report, and
    /// its receipt leaves the parent's watch in place.
    #[test]
    fn a_progress_letter_is_received_like_a_report_and_ends_no_watch() {
        let mut ledger = Ledger::default();
        let child = actor("child");
        let parent = actor("parent");
        super::super::watch::start(&mut ledger, &parent, &child, 1).unwrap();
        let plan = send(
            &mut ledger,
            &child,
            &parent,
            "plan",
            "Plan: core first, then web.",
            "progress",
            None,
            2,
        )
        .unwrap();
        assert!(!plan.waiting_answer);
        assert_eq!(pull(&ledger, &parent).unwrap().ids, vec![plan.id.clone()]);
        apply(
            &mut ledger,
            &parent,
            None,
            &Command::Confirm {
                ids: vec![plan.id.clone()],
            },
            3,
        )
        .unwrap();
        assert_eq!(ledger.letters[0].state, State::Delivered);
        assert!(ledger.letters[0].intake_confirmed());
        assert_eq!(ledger.watches.len(), 1, "a plan ends no watch");
        let restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
        assert_eq!(restored.letters[0].kind, "progress");
    }

    #[test]
    fn report_ends_only_its_parent_watch_on_hook_confirmation_and_replays_once() {
        let mut ledger = Ledger::default();
        let child = actor("child");
        let parent = actor("parent");
        let other = actor("other");
        super::super::watch::start(&mut ledger, &parent, &child, 1).unwrap();
        super::super::watch::start(&mut ledger, &other, &child, 1).unwrap();
        let report = send(
            &mut ledger,
            &child,
            &parent,
            "report",
            "done",
            "report",
            None,
            2,
        )
        .unwrap();
        assert!(!report.waiting_answer);
        assert_eq!(pull(&ledger, &parent).unwrap().ids, vec![report.id.clone()]);
        assert_eq!(ledger.watches.len(), 2);
        let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
        assert_eq!(
            send(
                &mut restored,
                &child,
                &parent,
                "report",
                "done",
                "report",
                None,
                3
            )
            .unwrap()
            .id,
            report.id
        );
        apply(
            &mut restored,
            &parent,
            None,
            &Command::Confirm {
                ids: vec![report.id.clone()],
            },
            4,
        )
        .unwrap();
        assert_eq!(restored.watches.len(), 1);
        assert_eq!(restored.watches[0].parent, other);
        assert_eq!(restored.letters[0].state, State::Delivered);
        assert!(pull(&restored, &parent).unwrap().ids.is_empty());
        // Starting another episode after delivery is explicitly supported.
        super::super::watch::start(&mut restored, &parent, &child, 5).unwrap();
        apply(
            &mut restored,
            &parent,
            None,
            &Command::Confirm {
                ids: vec![report.id],
            },
            6,
        )
        .unwrap();
        assert_eq!(restored.watches.len(), 2);
    }

    #[test]
    fn canceling_a_report_never_ends_a_watch() {
        let mut ledger = Ledger::default();
        let child = actor("child");
        let parent = actor("parent");
        super::super::watch::start(&mut ledger, &parent, &child, 1).unwrap();
        let report = send(
            &mut ledger,
            &child,
            &parent,
            "report",
            "done",
            "report",
            None,
            2,
        )
        .unwrap();
        apply(
            &mut ledger,
            &child,
            None,
            &Command::Cancel { id: report.id },
            3,
        )
        .unwrap();
        assert_eq!(ledger.watches.len(), 1);
    }

    #[test]
    fn ack_between_pull_and_confirmation_ends_only_matching_watch_once_across_restart() {
        let mut ledger = Ledger::default();
        let child = actor("child");
        let parent = actor("parent");
        let other = actor("other");
        super::super::watch::start(&mut ledger, &parent, &child, 1).unwrap();
        let unrelated = super::super::watch::start(&mut ledger, &other, &child, 1).unwrap();
        let report = send(
            &mut ledger,
            &child,
            &parent,
            "report",
            "done",
            "report",
            None,
            2,
        )
        .unwrap();
        let intake = pull(&ledger, &parent).unwrap();
        assert_eq!(intake.ids.as_slice(), std::slice::from_ref(&report.id));
        // An interruption after Pull leaves the same report and both watches.
        let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
        assert_eq!(pull(&restored, &parent).unwrap().ids, intake.ids);
        assert_eq!(restored.watches.len(), 2);
        let acknowledged = apply(
            &mut restored,
            &parent,
            None,
            &Command::Ack {
                id: report.id.clone(),
            },
            3,
        )
        .unwrap();
        assert_eq!(acknowledged["state"], "acknowledged");
        assert_eq!(acknowledged["hook_confirmed"], true);
        assert_eq!(restored.watches.len(), 1);
        assert_eq!(restored.watches[0].id, unrelated.id);
        assert!(pull(&restored, &parent).unwrap().ids.is_empty());
        // The interrupted hook's confirmation still answers and changes nothing.
        let mut restored: Ledger = serde_json::from_slice(&restored.bytes().unwrap()).unwrap();
        let rearmed = super::super::watch::start(&mut restored, &parent, &child, 4).unwrap();
        let before = restored.clone();
        let confirm = Command::Confirm { ids: intake.ids };
        assert_eq!(
            apply(&mut restored, &parent, None, &confirm, 5).unwrap()["confirmed"],
            json!([report.id.clone()])
        );
        assert_eq!(restored, before);
        assert!(restored.watches.iter().any(|watch| watch.id == rearmed.id));
        assert_eq!(
            send(
                &mut restored,
                &child,
                &parent,
                "report",
                "done",
                "report",
                None,
                7
            )
            .unwrap()
            .id,
            report.id
        );
        assert_eq!(restored.letters.len(), 1);
    }

    #[test]
    fn confirmation_then_ack_preserves_rearmed_watch_on_replay_after_restart() {
        let mut ledger = Ledger::default();
        let child = actor("child");
        let parent = actor("parent");
        super::super::watch::start(&mut ledger, &parent, &child, 1).unwrap();
        let report = send(
            &mut ledger,
            &child,
            &parent,
            "report",
            "done",
            "report",
            None,
            2,
        )
        .unwrap();
        let confirm = Command::Confirm {
            ids: pull(&ledger, &parent).unwrap().ids,
        };
        apply(&mut ledger, &parent, None, &confirm, 3).unwrap();
        assert!(ledger.watches.is_empty());
        let acknowledged = apply(
            &mut ledger,
            &parent,
            None,
            &Command::Ack { id: report.id },
            4,
        )
        .unwrap();
        assert_eq!(acknowledged["state"], "acknowledged");
        assert_eq!(acknowledged["hook_confirmed"], true);
        let rearmed = super::super::watch::start(&mut ledger, &parent, &child, 5).unwrap();
        let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
        let before = restored.clone();
        apply(&mut restored, &parent, None, &confirm, 6).unwrap();
        assert_eq!(restored, before);
        assert_eq!(restored.watches[0].id, rearmed.id);
    }

    #[test]
    fn an_acknowledgement_after_an_interrupted_pull_is_the_receipt() {
        for kind in ["request", "report"] {
            let mut ledger = Ledger::default();
            let sender = actor("sender");
            let recipient = actor("recipient");
            let letter = send(
                &mut ledger,
                &sender,
                &recipient,
                "once",
                "body",
                kind,
                None,
                1,
            )
            .unwrap();
            let intake = pull(&ledger, &recipient).unwrap();
            assert_eq!(intake.ids.as_slice(), std::slice::from_ref(&letter.id));
            apply(
                &mut ledger,
                &recipient,
                None,
                &Command::Ack { id: letter.id },
                2,
            )
            .unwrap();
            let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
            assert!(pull(&restored, &recipient).unwrap().ids.is_empty());
            let before = restored.clone();
            apply(
                &mut restored,
                &recipient,
                None,
                &Command::Confirm { ids: intake.ids },
                3,
            )
            .unwrap();
            assert_eq!(restored, before);
        }
    }

    #[test]
    fn actual_confirmation_after_cancel_or_deadline_ends_watch_once_and_survives_restart() {
        for cancel in [false, true] {
            let mut ledger = Ledger::default();
            let child = actor("child");
            let parent = actor("parent");
            super::super::watch::start(&mut ledger, &parent, &child, 1).unwrap();
            let report = send(
                &mut ledger,
                &child,
                &parent,
                "report",
                "done",
                "report",
                None,
                2,
            )
            .unwrap();
            let confirm = Command::Confirm {
                ids: pull(&ledger, &parent).unwrap().ids,
            };
            let now = 2 + super::super::DELIVERY_EXPIRY_MS;
            if cancel {
                assert_eq!(
                    apply(
                        &mut ledger,
                        &child,
                        None,
                        &Command::Cancel {
                            id: report.id.clone()
                        },
                        now
                    )
                    .unwrap()["state"],
                    "cancelled"
                );
            } else {
                assert!(ledger.expire(now));
                assert_eq!(
                    apply(
                        &mut ledger,
                        &parent,
                        None,
                        &Command::Show {
                            id: report.id.clone()
                        },
                        now
                    )
                    .unwrap()["state"],
                    "undelivered"
                );
            }
            assert_eq!(ledger.watches.len(), 1);
            assert!(pull(&ledger, &parent).unwrap().ids.is_empty());
            let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
            assert_eq!(
                apply(&mut restored, &parent, None, &confirm, now + 1).unwrap()["confirmed"],
                json!([report.id.clone()])
            );
            assert!(restored.watches.is_empty());
            let shown = apply(
                &mut restored,
                &parent,
                None,
                &Command::Show { id: report.id },
                now + 1,
            )
            .unwrap();
            assert_eq!(
                shown["state"],
                if cancel { "cancelled" } else { "delivered" }
            );
            assert_eq!(shown["hook_confirmed"], true);
            let rearmed =
                super::super::watch::start(&mut restored, &parent, &child, now + 2).unwrap();
            let mut restored: Ledger = serde_json::from_slice(&restored.bytes().unwrap()).unwrap();
            let before = restored.clone();
            apply(&mut restored, &parent, None, &confirm, now + 3).unwrap();
            assert_eq!(restored, before);
            assert_eq!(restored.watches[0].id, rearmed.id);
        }
    }

    /// A letter a build before this one acknowledged without a receipt.
    fn acknowledged_without_receipt(ledger: &mut Ledger, index: usize) {
        let letter = &mut ledger.letters[index];
        letter.state = State::Acknowledged;
        letter.hook_confirmed = Some(false);
        letter.finished_at_unix_ms = None;
    }

    #[test]
    fn reacknowledging_a_legacy_report_records_durable_intake_and_preserves_a_rearmed_watch() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("ledger.json");
        let mut ledger = Ledger::default();
        let child = actor("child");
        let parent = actor("parent");
        let other = actor("other");
        super::super::watch::start(&mut ledger, &parent, &child, 1).unwrap();
        let unrelated = super::super::watch::start(&mut ledger, &other, &child, 1).unwrap();
        let report = send(
            &mut ledger,
            &child,
            &parent,
            "legacy-report",
            "done",
            "report",
            None,
            2,
        )
        .unwrap();
        acknowledged_without_receipt(&mut ledger, 0);
        for index in 1..OPEN_LIMIT {
            send(
                &mut ledger,
                &child,
                &other,
                &format!("pending-{index}"),
                "body",
                "report",
                None,
                2,
            )
            .unwrap();
        }
        super::super::ledger::save(&path, &ledger).unwrap();
        let mut restored = super::super::ledger::load(&path).unwrap();
        let now = 2 + super::super::DELIVERY_EXPIRY_MS - 1;
        assert_eq!(
            pull(&restored, &parent).unwrap().ids,
            std::slice::from_ref(&report.id)
        );
        assert_eq!(
            send(
                &mut restored,
                &child,
                &other,
                "overflow",
                "body",
                "report",
                None,
                now,
            )
            .unwrap_err(),
            "capacity"
        );
        let ack = Command::Ack {
            id: report.id.clone(),
        };
        let acknowledged = apply(&mut restored, &parent, None, &ack, now).unwrap();
        assert_eq!(acknowledged["state"], "acknowledged");
        assert_eq!(acknowledged["hook_confirmed"], true);
        super::super::ledger::save(&path, &restored).unwrap();
        let mut restored = super::super::ledger::load(&path).unwrap();
        let shown = apply(
            &mut restored,
            &parent,
            None,
            &Command::Show { id: report.id },
            now,
        )
        .unwrap();
        assert_eq!(shown["hook_confirmed"], true);
        assert!(pull(&restored, &parent).unwrap().ids.is_empty());
        assert_eq!(restored.watches.len(), 1);
        assert_eq!(restored.watches[0].id, unrelated.id);
        send(
            &mut restored,
            &child,
            &other,
            "overflow",
            "body",
            "report",
            None,
            now,
        )
        .unwrap();
        let rearmed = super::super::watch::start(&mut restored, &parent, &child, now).unwrap();
        super::super::ledger::save(&path, &restored).unwrap();
        let mut restored = super::super::ledger::load(&path).unwrap();
        let before = restored.clone();
        apply(&mut restored, &parent, None, &ack, now + 1).unwrap();
        super::super::ledger::save(&path, &restored).unwrap();
        let restored = super::super::ledger::load(&path).unwrap();
        assert_eq!(restored, before);
        assert!(restored.watches.iter().any(|watch| watch.id == rearmed.id));
        assert!(pull(&restored, &parent).unwrap().ids.is_empty());
    }

    #[test]
    fn acknowledging_a_report_after_cancellation_or_expiry_does_not_record_intake() {
        for state in [
            State::Cancelled,
            State::Undelivered,
            State::Expired,
            State::Acknowledged,
        ] {
            let mut ledger = Ledger::default();
            let child = actor("child");
            let parent = actor("parent");
            super::super::watch::start(&mut ledger, &parent, &child, 1).unwrap();
            let report = send(
                &mut ledger,
                &child,
                &parent,
                "report",
                "done",
                "report",
                None,
                2,
            )
            .unwrap();
            acknowledged_without_receipt(&mut ledger, 0);
            let deadline = 2 + super::super::DELIVERY_EXPIRY_MS;
            if state == State::Acknowledged {
                assert!(ledger.expire(deadline));
            } else {
                ledger.letters[0].state = state;
                ledger.letters[0].finished_at_unix_ms = Some(deadline);
            }
            let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
            let before = restored.clone();
            apply(
                &mut restored,
                &parent,
                None,
                &Command::Ack { id: report.id },
                deadline,
            )
            .unwrap();
            assert_eq!(restored, before, "{state:?}");
            assert!(
                pull(&restored, &parent).unwrap().ids.is_empty(),
                "{state:?}"
            );
            assert_eq!(restored.watches.len(), 1, "{state:?}");
        }
    }

    #[test]
    fn a_bell_turn_hands_over_pending_letters_before_an_acknowledged_backlog() {
        let mut ledger = Ledger::default();
        let recipient = actor("recipient");
        pending_for(&mut ledger, &recipient, 7);
        for index in 0..6 {
            acknowledged_without_receipt(&mut ledger, index);
        }
        let fresh = ledger.letters[6].id.clone();
        ring(&mut ledger, 6, LINE);
        let intake = pull_for(&mut ledger, &recipient, LINE);
        assert_eq!(intake.ids.len(), 7);
        assert_eq!(intake.ids[0], fresh, "the letter the bell rang for");
        assert_eq!(
            intake.ids[1], ledger.letters[0].id,
            "then the oldest backlog"
        );
        assert_eq!(intake.remaining, 0);
    }

    #[test]
    fn an_acknowledged_letter_without_a_receipt_stops_awaiting_intake_at_its_deadline() {
        let mut ledger = Ledger::default();
        let sender = actor("sender");
        let recipient = actor("recipient");
        for index in 0..OPEN_LIMIT {
            let kind = if index == 0 { "block" } else { "report" };
            send(
                &mut ledger,
                &sender,
                &recipient,
                &format!("receipt-{index}"),
                "body",
                kind,
                None,
                1,
            )
            .unwrap();
            acknowledged_without_receipt(&mut ledger, index);
        }
        let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
        let deadline = 1 + super::super::DELIVERY_EXPIRY_MS;
        assert!(!restored.expire(deadline - 1));
        assert_eq!(
            send(
                &mut restored,
                &sender,
                &recipient,
                "overflow",
                "body",
                "report",
                None,
                deadline - 1
            )
            .unwrap_err(),
            "capacity"
        );
        // One sender's whole open backlog is one bundle: its block and its
        // newest three reports in full, the rest folded, all confirmable.
        assert_eq!(pull(&restored, &recipient).unwrap().ids.len(), OPEN_LIMIT);
        assert!(restored.expire(deadline));
        assert!(pull(&restored, &recipient).unwrap().ids.is_empty());
        // A block still awaiting its reply stays open; the reports do not.
        assert_eq!(restored.letters.iter().filter(|l| l.open()).count(), 1);
        let report = restored.letters[1].id.clone();
        let shown = apply(
            &mut restored,
            &recipient,
            None,
            &Command::Show { id: report },
            deadline,
        )
        .unwrap();
        assert_eq!(shown["state"], "acknowledged");
        assert!(shown["hook_confirmed"].is_null());
        assert_eq!(shown["finished_at_unix_ms"], deadline);
        send(
            &mut restored,
            &sender,
            &recipient,
            "overflow",
            "body",
            "report",
            None,
            deadline,
        )
        .unwrap();
        let reloaded: Ledger = serde_json::from_slice(&restored.bytes().unwrap()).unwrap();
        assert_eq!(reloaded, restored);
        assert!(!restored.clone().expire(deadline + 1));
    }

    #[test]
    fn remote_letters_use_the_same_durable_intent_and_native_session_boundary() {
        let mut ledger = Ledger::default();
        let local = actor("local");
        let mut remote = actor("remote:device:pane:target");
        remote.device_id = "device".into();
        let letter = send(
            &mut ledger,
            &local,
            &remote,
            "remote",
            "body",
            "block",
            None,
            1,
        )
        .unwrap();
        assert!(letter.waiting_answer);
        let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
        assert_eq!(
            pull(&restored, &remote).unwrap().ids,
            vec![letter.id.clone()]
        );
        let mut replaced = remote.clone();
        replaced.session = Some("replacement".into());
        assert!(pull(&restored, &replaced).unwrap().ids.is_empty());
        apply(
            &mut restored,
            &remote,
            None,
            &Command::Confirm {
                ids: vec![letter.id.clone()],
            },
            2,
        )
        .unwrap();
        assert!(pull(&restored, &remote).unwrap().ids.is_empty());
        assert_eq!(
            send(
                &mut restored,
                &local,
                &remote,
                "remote",
                "body",
                "block",
                None,
                3
            )
            .unwrap()
            .id,
            letter.id
        );
    }

    #[test]
    fn the_open_limit_rejects_without_removing_existing_letters() {
        let mut ledger = Ledger::default();
        let a = actor("a");
        let b = actor("b");
        for index in 0..OPEN_LIMIT {
            send(
                &mut ledger,
                &a,
                &b,
                &format!("open-{index}"),
                "body",
                "request",
                None,
                1,
            )
            .unwrap();
        }
        assert_eq!(
            send(&mut ledger, &a, &b, "overflow", "body", "request", None, 1).unwrap_err(),
            "capacity"
        );
        assert_eq!(ledger.letters.len(), OPEN_LIMIT);
        assert!(!ledger.cleanup(RETENTION_MS * 2));
        for letter in &mut ledger.letters {
            letter.state = State::Cancelled;
            letter.waiting_answer = false;
            letter.finished_at_unix_ms = Some(1);
        }
        assert!(!ledger.cleanup(RETENTION_MS));
        assert!(ledger.cleanup(RETENTION_MS + 1));
        assert!(ledger.letters.is_empty());
    }

    /// A report the recipient took in, finished at `finished`.
    fn finished_report(ledger: &mut Ledger, intent: &str, finished: u64) -> String {
        let (a, b) = (actor("a"), actor("b"));
        let letter = send(ledger, &a, &b, intent, "body", "report", None, 1).unwrap();
        let stored = ledger
            .letters
            .iter_mut()
            .find(|stored| stored.id == letter.id)
            .unwrap();
        stored.state = State::Acknowledged;
        stored.hook_confirmed = Some(true);
        stored.finished_at_unix_ms = Some(finished);
        letter.id
    }

    #[test]
    fn a_finished_letters_intent_is_kept_for_a_day_from_its_finish() {
        let (a, b) = (actor("a"), actor("b"));
        let mut ledger = Ledger::default();
        let first = send(&mut ledger, &a, &b, "retry", "body", "request", None, 1).unwrap();
        // A request that waited most of a day is open until it is answered.
        let answered = RETENTION_MS - 1;
        apply(
            &mut ledger,
            &b,
            None,
            &Command::Confirm {
                ids: vec![first.id.clone()],
            },
            answered,
        )
        .unwrap();
        apply(
            &mut ledger,
            &b,
            None,
            &Command::Reply {
                id: first.id.clone(),
                intent: "answer".into(),
                body: "done".into(),
            },
            answered,
        )
        .unwrap();
        let finished = ledger.letters[0].finished_at_unix_ms.unwrap();
        let again = |ledger: &mut Ledger, now: u64| {
            send(ledger, &a, &b, "retry", "body", "request", None, now)
                .unwrap()
                .id
        };
        assert_eq!(again(&mut ledger, finished + RETENTION_MS - 1), first.id);
        assert_ne!(again(&mut ledger, finished + RETENTION_MS), first.id);
    }

    #[test]
    fn a_full_total_drops_the_oldest_finished_letter_and_keeps_every_open_one() {
        let (a, b) = (actor("a"), actor("b"));
        let mut ledger = Ledger::default();
        let open: Vec<String> = (0..100)
            .map(|index| {
                send(
                    &mut ledger,
                    &a,
                    &b,
                    &format!("open-{index}"),
                    "body",
                    "request",
                    None,
                    1,
                )
                .unwrap()
                .id
            })
            .collect();
        // Later letters finished earlier, so the order is the finish time's,
        // not the position's.
        let finished: Vec<String> = (0..LETTER_LIMIT - open.len())
            .map(|index| {
                finished_report(
                    &mut ledger,
                    &format!("done-{index}"),
                    100_000 - index as u64,
                )
            })
            .collect();
        assert_eq!(ledger.letters.len(), LETTER_LIMIT);
        let sequence = ledger.next_id;
        let sent = send(
            &mut ledger,
            &a,
            &b,
            "fresh",
            "body",
            "request",
            None,
            200_000,
        )
        .unwrap();
        assert_eq!(ledger.letters.len(), LETTER_LIMIT);
        assert_eq!(ledger.next_id, sequence + 1);
        assert_eq!(ledger.letters.last().unwrap().id, sent.id);
        let kept: Vec<&str> = ledger
            .letters
            .iter()
            .map(|letter| letter.id.as_str())
            .collect();
        assert!(open.iter().all(|id| kept.contains(&id.as_str())));
        assert!(!kept.contains(&finished.last().unwrap().as_str()));
        assert!(kept.contains(&finished[0].as_str()));
        ledger.validate().unwrap();
        // The next one takes the next oldest.
        send(
            &mut ledger,
            &a,
            &b,
            "second",
            "body",
            "request",
            None,
            200_001,
        )
        .unwrap();
        let kept: Vec<&str> = ledger
            .letters
            .iter()
            .map(|letter| letter.id.as_str())
            .collect();
        assert!(!kept.contains(&finished[finished.len() - 2].as_str()));
    }

    #[test]
    fn a_capacity_refusal_is_logged_with_the_cap_that_fired_and_never_the_body() {
        let (a, b) = (actor("a"), actor("b"));
        let secret = "secret-body-text";
        let refuse = |ledger: &mut Ledger, body: &str| {
            let (result, records) = crate::diagnostics::capture(|| {
                send(ledger, &a, &b, "refused", body, "request", None, 2)
            });
            assert_eq!(result.unwrap_err(), "capacity");
            assert_eq!(records.len(), 1);
            assert!(!records[0].to_string().contains(secret));
            records[0].clone()
        };

        // The open cap holds although finished letters could be dropped.
        let mut ledger = Ledger::default();
        for index in OPEN_LIMIT..LETTER_LIMIT {
            finished_report(&mut ledger, &format!("done-{index}"), 1);
        }
        for index in 0..OPEN_LIMIT {
            send(
                &mut ledger,
                &a,
                &b,
                &format!("open-{index}"),
                "body",
                "request",
                None,
                1,
            )
            .unwrap();
        }
        let before = ledger.clone();
        let record = refuse(&mut ledger, secret);
        assert_eq!(record["kind"], "letter.capacity_refused");
        assert_eq!(record["cap"], "open");
        assert_eq!(record["sender_pane"], "a");
        assert_eq!(record["recipient_pane"], "b");
        assert_eq!(record["letters"], LETTER_LIMIT);
        assert_eq!(record["open"], OPEN_LIMIT);
        assert_eq!(ledger, before);

        // A full total with nothing finished to drop.
        let mut ledger = Ledger::default();
        let open = send(&mut ledger, &a, &b, "open", "body", "request", None, 1).unwrap();
        ledger.letters = vec![open; LETTER_LIMIT];
        let before = ledger.clone();
        assert_eq!(refuse(&mut ledger, secret)["cap"], "total");
        assert_eq!(ledger, before);

        // A body over the limit.
        let mut ledger = Ledger::default();
        let long = format!("{secret}{}", "x".repeat(BODY_LIMIT));
        let record = refuse(&mut ledger, &long);
        assert_eq!(record["cap"], "body");
        assert_eq!(record["body_bytes"], long.len());
        assert!(ledger.letters.is_empty());
    }

    /// A request the recipient took in and has not answered.
    fn awaiting_reply(
        ledger: &mut Ledger,
        from: &Actor,
        to: &Actor,
        intent: &str,
        at: u64,
    ) -> Letter {
        let letter = send(ledger, from, to, intent, "body", "request", None, at).unwrap();
        apply(
            ledger,
            to,
            None,
            &Command::Confirm {
                ids: vec![letter.id.clone()],
            },
            at,
        )
        .unwrap();
        ledger
            .letters
            .iter()
            .find(|l| l.id == letter.id)
            .unwrap()
            .clone()
    }

    fn letter_of<'a>(ledger: &'a Ledger, id: &str) -> &'a Letter {
        ledger.letters.iter().find(|l| l.id == id).unwrap()
    }

    #[test]
    fn an_unanswered_request_stops_waiting_exactly_at_the_deadline_and_says_why() {
        let (a, b) = (actor("a"), actor("b"));
        let mut ledger = Ledger::default();
        let sent = 1_000;
        let letter = awaiting_reply(&mut ledger, &a, &b, "ask", sent);
        let deadline = sent + super::super::ANSWER_WAIT_MS;
        assert!(!ledger.end_overdue_answer_waits(deadline - 1));
        let open = letter_of(&ledger, &letter.id);
        assert!(open.waiting_answer && open.open() && open.answer_wait_ended.is_none());
        assert!(ledger.end_overdue_answer_waits(deadline));
        let shown = apply(
            &mut ledger,
            &a,
            None,
            &Command::Show {
                id: letter.id.clone(),
            },
            deadline,
        )
        .unwrap();
        assert_eq!(shown["waiting_answer"], false);
        assert_eq!(shown["answer_wait_ended"], "deadline");
        assert_eq!(shown["finished_at_unix_ms"], deadline);
        assert_eq!(shown["state"], "delivered");
        assert!(!letter_of(&ledger, &letter.id).open());
        ledger.validate().unwrap();
        // A second pass neither reports a close nor moves the finish time.
        assert!(!ledger.end_overdue_answer_waits(deadline + 1));
        assert_eq!(
            letter_of(&ledger, &letter.id).finished_at_unix_ms,
            Some(deadline)
        );
    }

    #[test]
    fn the_deadline_leaves_answered_cancelled_and_undelivered_letters_alone() {
        let (a, b) = (actor("a"), actor("b"));
        let mut ledger = Ledger::default();
        let answered = awaiting_reply(&mut ledger, &a, &b, "answered", 1);
        apply(
            &mut ledger,
            &b,
            None,
            &Command::Reply {
                id: answered.id.clone(),
                intent: "answer".into(),
                body: "done".into(),
            },
            2,
        )
        .unwrap();
        let cancelled = awaiting_reply(&mut ledger, &a, &b, "cancelled", 1);
        apply(
            &mut ledger,
            &a,
            None,
            &Command::Cancel {
                id: cancelled.id.clone(),
            },
            2,
        )
        .unwrap();
        let unreceived = send(
            &mut ledger,
            &a,
            &b,
            "unreceived",
            "body",
            "request",
            None,
            1,
        )
        .unwrap();
        // The deadline itself, while the letters finished at 2 are still kept.
        let late = 1 + super::super::ANSWER_WAIT_MS;
        assert!(ledger.expire(late));
        assert!(!ledger.end_overdue_answer_waits(late));
        for (id, state) in [
            (&answered.id, State::Delivered),
            (&cancelled.id, State::Cancelled),
            (&unreceived.id, State::Undelivered),
        ] {
            let letter = letter_of(&ledger, id);
            assert_eq!(letter.state, state);
            assert!(letter.answer_wait_ended.is_none());
        }
    }

    #[test]
    fn ending_a_registration_ends_the_waits_it_sent_or_received_and_no_others() {
        let (a, b, c) = (actor("a"), actor("b"), actor("c"));
        let mut ledger = Ledger::default();
        let to_b = awaiting_reply(&mut ledger, &a, &b, "to-b", 1);
        let from_b = awaiting_reply(&mut ledger, &b, &c, "from-b", 1);
        let elsewhere = awaiting_reply(&mut ledger, &a, &c, "elsewhere", 1);
        let report = send(&mut ledger, &a, &b, "report", "body", "report", None, 1).unwrap();
        assert!(ledger.end_answer_waits_of(&b, 9));
        for id in [&to_b.id, &from_b.id] {
            let letter = letter_of(&ledger, id);
            assert!(!letter.waiting_answer && !letter.open());
            assert_eq!(letter.answer_wait_ended, Some(AnswerWaitEnd::PartyEnded));
            assert_eq!(letter.finished_at_unix_ms, Some(9));
        }
        assert!(letter_of(&ledger, &elsewhere.id).waiting_answer);
        assert!(letter_of(&ledger, &report.id).answer_wait_ended.is_none());
        assert!(!ledger.end_answer_waits_of(&b, 10));
        ledger.validate().unwrap();
    }

    #[test]
    fn only_the_exact_pane_device_and_session_that_ended_closes_a_wait() {
        let (a, b) = (actor("a"), actor("b"));
        let mut ledger = Ledger::default();
        awaiting_reply(&mut ledger, &a, &b, "ask", 1);
        let restarted_session = Actor {
            session: Some("another-session".into()),
            ..b.clone()
        };
        let other_device = Actor {
            device_id: "another-device".into(),
            ..b.clone()
        };
        let other_pane = Actor {
            pane_id: "another-pane".into(),
            ..b.clone()
        };
        let renamed_only = Actor {
            name: "b".into(),
            pane_id: "elsewhere".into(),
            ..b.clone()
        };
        for stranger in [
            &restarted_session,
            &other_device,
            &other_pane,
            &renamed_only,
        ] {
            assert!(!ledger.end_answer_waits_of(stranger, 5), "{stranger:?}");
        }
        assert!(ledger.letters[0].waiting_answer);
        assert!(ledger.end_answer_waits_of(&b, 5));
    }

    #[test]
    fn a_request_not_yet_taken_in_is_left_to_the_delivery_deadline_when_its_party_ends() {
        let (a, b) = (actor("a"), actor("b"));
        let mut ledger = Ledger::default();
        let letter = send(&mut ledger, &a, &b, "ask", "body", "request", None, 1).unwrap();
        assert!(!ledger.end_answer_waits_of(&a, 5));
        assert!(!ledger.end_answer_waits_of(&b, 5));
        let pending = letter_of(&ledger, &letter.id);
        assert!(pending.waiting_answer && pending.answer_wait_ended.is_none());
        // Never collected, it ends as undelivered, as before this rule.
        assert!(ledger.expire(1 + super::super::DELIVERY_EXPIRY_MS));
        let undelivered = letter_of(&ledger, &letter.id);
        assert_eq!(undelivered.state, State::Undelivered);
        assert!(!undelivered.open() && undelivered.answer_wait_ended.is_none());
        ledger.validate().unwrap();
    }

    #[test]
    fn a_full_mailbox_accepts_a_letter_again_once_the_waits_end() {
        let (a, b, c) = (actor("a"), actor("b"), actor("c"));
        let mut ledger = Ledger::default();
        for index in 0..OPEN_LIMIT {
            awaiting_reply(&mut ledger, &a, &b, &format!("open-{index}"), 1);
        }
        let refused = |ledger: &mut Ledger, intent: &str| {
            send(ledger, &a, &c, intent, "body", "request", None, 2)
        };
        assert_eq!(refused(&mut ledger, "first").unwrap_err(), "capacity");
        ledger.end_answer_waits_of(&b, 2);
        assert!(refused(&mut ledger, "second").is_ok());
        // The cap itself is unchanged: filling it again is refused again.
        for index in 1..OPEN_LIMIT {
            refused(&mut ledger, &format!("again-{index}")).unwrap();
        }
        assert_eq!(refused(&mut ledger, "last").unwrap_err(), "capacity");
    }

    #[test]
    fn a_ledger_written_before_the_field_loads_and_only_old_waits_close() {
        let (a, b) = (actor("a"), actor("b"));
        let mut ledger = Ledger::default();
        let now = 100 * super::super::ANSWER_WAIT_MS;
        let old = awaiting_reply(&mut ledger, &a, &b, "old", now - 50 * 60 * 60_000);
        let fresh = awaiting_reply(&mut ledger, &a, &b, "fresh", now - 12 * 60 * 60_000);
        let mut written = serde_json::to_value(&ledger).unwrap();
        for letter in written["letters"].as_array_mut().unwrap() {
            letter.as_object_mut().unwrap().remove("answer_wait_ended");
        }
        let mut loaded: Ledger = serde_json::from_value(written).unwrap();
        loaded.validate().unwrap();
        loaded.expire(now);
        assert!(loaded.end_overdue_answer_waits(now));
        assert_eq!(
            letter_of(&loaded, &old.id).answer_wait_ended,
            Some(AnswerWaitEnd::Deadline)
        );
        assert!(letter_of(&loaded, &fresh.id).waiting_answer);
        loaded.validate().unwrap();
    }

    #[test]
    fn a_waiting_letter_cannot_carry_a_reason_for_having_stopped() {
        let (a, b) = (actor("a"), actor("b"));
        let mut ledger = Ledger::default();
        let letter = awaiting_reply(&mut ledger, &a, &b, "ask", 1);
        ledger
            .letters
            .iter_mut()
            .find(|l| l.id == letter.id)
            .unwrap()
            .answer_wait_ended = Some(AnswerWaitEnd::Deadline);
        assert_eq!(ledger.validate().unwrap_err(), "ledger_unavailable");
    }

    /// `sender` writes one letter of `kind` to `recipient` for each body,
    /// a millisecond apart from `from`, and returns their ids.
    fn letters_from(
        ledger: &mut Ledger,
        sender: &Actor,
        recipient: &Actor,
        kinds: &[&str],
        from: u64,
    ) -> Vec<String> {
        kinds
            .iter()
            .enumerate()
            .map(|(index, kind)| {
                let at = from + index as u64;
                send(
                    ledger,
                    sender,
                    recipient,
                    &format!("{}-{at}", sender.name),
                    &format!("body of {} at {at}", sender.name),
                    kind,
                    None,
                    at,
                )
                .unwrap()
                .id
            })
            .collect()
    }

    fn bundle_for(ledger: &Ledger, recipient: &Actor) -> Intake {
        pull_in(
            ledger,
            recipient,
            InterfaceLanguage::Korean,
            Bundling::PerSender,
        )
        .unwrap()
    }

    #[test]
    fn a_sender_with_five_progress_letters_shows_its_newest_three_and_folds_two() {
        let mut ledger = Ledger::default();
        let (worker, conductor) = (actor("worker"), actor("conductor"));
        let ids = letters_from(&mut ledger, &worker, &conductor, &["progress"; 5], 1);
        let intake = bundle_for(&ledger, &conductor);
        assert!(intake.context.contains(
            "Hide letters from worker (codex) 외 2통: hide request show <id> (letter-2, letter-1)"
        ));
        for at in 1..=2 {
            assert!(!intake.context.contains(&format!("body of worker at {at}")));
        }
        for at in 3..=5 {
            assert!(
                intake
                    .context
                    .contains(&format!("body of worker at {at}\n"))
            );
        }
        // The folded letters are confirmed with the shown ones.
        assert_eq!(intake.ids, ids);
        assert_eq!(intake.remaining, 0);
    }

    #[test]
    fn a_sender_with_two_letters_is_shown_whole_with_no_fold_line() {
        let mut ledger = Ledger::default();
        let (worker, conductor) = (actor("worker"), actor("conductor"));
        let ids = letters_from(&mut ledger, &worker, &conductor, &["progress", "report"], 1);
        let intake = bundle_for(&ledger, &conductor);
        assert!(!intake.context.contains("Hide letters from"));
        assert!(!intake.context.contains("외 "));
        assert_eq!(intake.context.matches("\nHide letter ").count(), 2);
        assert_eq!(intake.ids, ids);
    }

    #[test]
    fn a_request_among_ten_progress_letters_is_never_folded() {
        let mut ledger = Ledger::default();
        let (worker, conductor) = (actor("worker"), actor("conductor"));
        let mut kinds = vec!["request"];
        kinds.extend(["progress"; 10]);
        let ids = letters_from(&mut ledger, &worker, &conductor, &kinds, 1);
        let intake = bundle_for(&ledger, &conductor);
        // The oldest letter is the question, shown in full.
        assert!(intake.context.contains(&format!(
            "\nHide letter {} from worker (codex) [request]\nbody of worker at 1\n",
            ids[0]
        )));
        assert!(intake.context.contains("외 7통"));
        assert_eq!(intake.context.matches("\nHide letter ").count(), 4);
        assert_eq!(intake.ids.len(), 11);
    }

    #[test]
    fn one_confirmation_takes_in_folded_letters_and_ends_a_folded_reports_watch() {
        let mut ledger = Ledger::default();
        let (child, parent) = (actor("child"), actor("parent"));
        super::super::watch::start(&mut ledger, &parent, &child, 1).unwrap();
        let mut kinds = vec!["report"];
        kinds.extend(["progress"; 4]);
        let ids = letters_from(&mut ledger, &child, &parent, &kinds, 2);
        let intake = bundle_for(&ledger, &parent);
        // The report is the oldest of five, so it is folded.
        assert!(!intake.context.contains("[report]"));
        assert!(intake.context.contains("외 2통"));
        apply(
            &mut ledger,
            &parent,
            None,
            &Command::Confirm {
                ids: intake.ids.clone(),
            },
            20,
        )
        .unwrap();
        for id in &ids {
            let letter = ledger
                .letters
                .iter()
                .find(|letter| letter.id == *id)
                .unwrap();
            assert_eq!(letter.state, State::Delivered, "{id}");
            assert_eq!(letter.hook_confirmed, Some(true), "{id}");
        }
        assert!(
            ledger.watches.is_empty(),
            "the folded report ended the watch"
        );
        assert!(bundle_for(&ledger, &parent).ids.is_empty());
    }

    #[test]
    fn mixed_senders_each_keep_their_newest_three() {
        let mut ledger = Ledger::default();
        let conductor = actor("conductor");
        let (a, b) = (actor("worker-a"), actor("worker-b"));
        let a_ids = letters_from(&mut ledger, &a, &conductor, &["progress"; 5], 1);
        let b_ids = letters_from(&mut ledger, &b, &conductor, &["progress"; 4], 10);
        let intake = bundle_for(&ledger, &conductor);
        assert!(intake.context.contains(&format!(
            "Hide letters from worker-a (codex) 외 2통: hide request show <id> ({}, {})",
            a_ids[1], a_ids[0]
        )));
        assert!(intake.context.contains(&format!(
            "Hide letters from worker-b (codex) 외 1통: hide request show <id> ({})",
            b_ids[0]
        )));
        assert_eq!(intake.context.matches("\nHide letter ").count(), 6);
        // Each sender's letters stay together, the earlier sender first.
        let a_last = intake.context.find("body of worker-a at 5").unwrap();
        let b_first = intake.context.find("Hide letters from worker-b").unwrap();
        assert!(a_last < b_first);
        assert_eq!(intake.ids.len(), 9);
    }

    #[test]
    fn letters_past_the_size_cap_are_neither_shown_nor_confirmed() {
        let mut ledger = Ledger::default();
        let conductor = actor("conductor");
        let (asker, worker) = (actor("asker"), actor("worker"));
        let question = send(
            &mut ledger,
            &asker,
            &conductor,
            "big-question",
            &"q".repeat(BODY_LIMIT),
            "request",
            None,
            1,
        )
        .unwrap()
        .id;
        let progress = letters_from(&mut ledger, &worker, &conductor, &["progress"; 5], 2);
        let intake = bundle_for(&ledger, &conductor);
        assert!(intake.context.len() <= HOOK_LIMIT);
        // The question fills the cap; the worker's letters, shown neither in
        // full nor on a fold line, wait for the next handover.
        assert_eq!(intake.ids, vec![question]);
        assert_eq!(intake.remaining, 5);
        assert!(!intake.context.contains("Hide letters from worker"));
        assert!(
            intake
                .context
                .contains("대기 5통 더 있음. hide inbox로 확인하세요.")
        );
        apply(
            &mut ledger,
            &conductor,
            None,
            &Command::Confirm { ids: intake.ids },
            20,
        )
        .unwrap();
        for id in &progress {
            let letter = ledger
                .letters
                .iter()
                .find(|letter| letter.id == *id)
                .unwrap();
            assert_eq!(letter.state, State::Pending, "{id}");
        }
        // The next handover folds them as the first would have.
        let next = bundle_for(&ledger, &conductor);
        assert!(next.context.contains("외 2통"));
        assert_eq!(next.ids, progress);
    }

    #[test]
    fn inbox_folds_like_a_handover_and_one_ack_takes_the_bundle_but_not_a_later_letter() {
        let mut ledger = Ledger::default();
        let worker = actor("worker");
        let mut reader = actor("reader");
        reader.kind = "cursor".into();
        assert!(!prompt_hook(&reader.kind));
        super::super::watch::start(&mut ledger, &reader, &worker, 1).unwrap();
        let mut kinds = vec!["report"];
        kinds.extend(["progress"; 5]);
        let ids = letters_from(&mut ledger, &worker, &reader, &kinds, 2);
        let inbox = apply(
            &mut ledger,
            &reader,
            None,
            &Command::Inbox { bundle: true },
            20,
        )
        .unwrap();
        assert_eq!(inbox["letters"].as_array().unwrap().len(), 3);
        assert_eq!(
            inbox["folded"],
            json!([{
                "sender": "worker",
                "sender_kind": "codex",
                "ids": ids[..3],
            }])
        );
        let all: Vec<&str> = ids.iter().map(String::as_str).collect();
        let bundle = bundle_token(&all).unwrap();
        assert!(bundle.starts_with(&format!("{}~", ids[5])));
        assert_eq!(inbox["ack_command"], format!("hide inbox --ack {bundle}"));
        let later = letters_from(&mut ledger, &worker, &reader, &["progress"], 30);
        let acked = apply(
            &mut ledger,
            &reader,
            None,
            &Command::AckBundle {
                bundle: bundle.clone(),
            },
            40,
        )
        .unwrap();
        assert_eq!(acked, json!({"bundle": bundle, "acknowledged": 6}));
        for id in &ids {
            let letter = ledger
                .letters
                .iter()
                .find(|letter| letter.id == *id)
                .unwrap();
            assert_eq!(letter.state, State::Acknowledged, "{id}");
        }
        assert!(
            ledger.watches.is_empty(),
            "the folded report ended the watch"
        );
        let left = apply(
            &mut ledger,
            &reader,
            None,
            &Command::Inbox { bundle: true },
            50,
        )
        .unwrap();
        assert_eq!(left["letters"][0]["id"], later[0]);
        let next = bundle_token(&[later[0].as_str()]).unwrap();
        assert_eq!(left["ack_command"], format!("hide inbox --ack {next}"));
        // Another agent's letter cannot stand for the caller's bundle.
        assert_eq!(
            apply(
                &mut ledger,
                &worker,
                None,
                &Command::AckBundle { bundle: next },
                60,
            )
            .unwrap_err(),
            "recipient_required"
        );
    }

    #[test]
    fn a_bundle_ack_takes_only_the_letters_its_inbox_listed() {
        let mut ledger = Ledger::default();
        let mut reader = actor("reader");
        reader.kind = "cursor".into();
        let (a, b) = (actor("worker-a"), actor("worker-b"));
        // Interleaved senders: the inbox lists worker-a's letters first,
        // though worker-b's oldest came before worker-a's newest.
        let first_b = letters_from(&mut ledger, &b, &reader, &["progress"], 1);
        let a_ids = letters_from(&mut ledger, &a, &reader, &["progress"; 2], 2);
        let inbox = apply(
            &mut ledger,
            &reader,
            None,
            &Command::Inbox { bundle: true },
            10,
        )
        .unwrap();
        let ack: String = serde_json::from_value(inbox["ack_command"].clone()).unwrap();
        let bundle = ack.strip_prefix("hide inbox --ack ").unwrap().to_owned();
        // A token naming only some of the letters below its newest, as an
        // inbox that left one out would print, takes nothing.
        let partial = bundle_token(&[a_ids[0].as_str(), a_ids[1].as_str()]).unwrap();
        assert_eq!(
            apply(
                &mut ledger,
                &reader,
                None,
                &Command::AckBundle { bundle: partial },
                11,
            )
            .unwrap_err(),
            "bundle_changed"
        );
        // A letter of the bundle taken some other way changes the set too.
        let mut taken = ledger.clone();
        apply(
            &mut taken,
            &reader,
            None,
            &Command::Ack {
                id: first_b[0].clone(),
            },
            11,
        )
        .unwrap();
        assert_eq!(
            apply(
                &mut taken,
                &reader,
                None,
                &Command::AckBundle {
                    bundle: bundle.clone(),
                },
                12,
            )
            .unwrap_err(),
            "bundle_changed"
        );
        assert!(
            ledger
                .letters
                .iter()
                .all(|letter| letter.state == State::Pending)
        );
        let acked = apply(
            &mut ledger,
            &reader,
            None,
            &Command::AckBundle { bundle },
            13,
        )
        .unwrap();
        assert_eq!(acked["acknowledged"], 3);
    }

    /// A connected device whose kit is older than this build sends no
    /// `bundle` on its pull and accepts at most five ids to confirm; it is
    /// handed its letters as before, so they keep flowing five at a time.
    #[test]
    fn an_older_kit_is_handed_five_letters_at_a_time_and_never_stuck() {
        let mut ledger = Ledger::default();
        let (worker, conductor) = (actor("worker"), actor("conductor"));
        let ids = letters_from(&mut ledger, &worker, &conductor, &["progress"; 7], 1);
        let old_kit = Command::from_wire(json!({"op": "pull"})).unwrap();
        assert_eq!(Bundling::of(&old_kit), Bundling::Legacy);
        let intake = pull_in(
            &ledger,
            &conductor,
            InterfaceLanguage::Korean,
            Bundling::of(&old_kit),
        )
        .unwrap();
        assert_eq!(intake.ids, ids[..5]);
        assert!(hide_agent_hooks::delivery::valid_letter_ids(&intake.ids));
        assert!(!intake.context.contains("Hide letters from"));
        assert_eq!(intake.remaining, 2);
        apply(
            &mut ledger,
            &conductor,
            None,
            &Command::Confirm { ids: intake.ids },
            20,
        )
        .unwrap();
        let next = pull_in(
            &ledger,
            &conductor,
            InterfaceLanguage::Korean,
            Bundling::Legacy,
        )
        .unwrap();
        assert_eq!(next.ids, ids[5..]);
        assert_eq!(next.remaining, 0);
        // Its `hide inbox` has no `--ack`, so each letter keeps its own ack.
        let mut reader = actor("reader");
        reader.kind = "cursor".into();
        letters_from(&mut ledger, &worker, &reader, &["progress"; 4], 30);
        let old_inbox = Command::from_wire(json!({"op": "inbox"})).unwrap();
        let inbox = apply(&mut ledger, &reader, None, &old_inbox, 40).unwrap();
        let entries = inbox.as_array().unwrap();
        assert_eq!(entries.len(), 4);
        for entry in entries {
            assert_eq!(
                entry["ack_command"],
                format!("hide request ack {}", entry["id"].as_str().unwrap())
            );
        }
    }

    /// A hook's confirmation reaches the ledger as `hide inbox --confirm`'s
    /// does, whether its hook sent it over the hook's own request or a `hide`
    /// it started (with-hide/hide-private#663): only the recipient's own pane
    /// and session take its letters as received. The sender, a pane that is
    /// no party to the letter and a later session in the recipient's pane
    /// change nothing.
    #[test]
    fn a_confirmation_is_taken_only_from_the_recipients_own_pane_and_session() {
        let mut ledger = Ledger::default();
        let sender = actor("sender");
        let recipient = actor("recipient");
        let letter = send(
            &mut ledger,
            &sender,
            &recipient,
            "intent",
            "body",
            "request",
            None,
            1,
        )
        .unwrap();
        assert_eq!(
            pull(&ledger, &recipient).unwrap().ids,
            vec![letter.id.clone()]
        );
        let confirm = Command::Confirm {
            ids: vec![letter.id.clone()],
        };
        let mut later = recipient.clone();
        later.session = Some("session-later".into());
        let before = ledger.clone();
        for (caller, refusal) in [
            (sender, "recipient_required"),
            (actor("bystander"), "letter_unavailable"),
            (later, "letter_unavailable"),
        ] {
            assert_eq!(
                apply(&mut ledger, &caller, None, &confirm, 2).unwrap_err(),
                refusal,
                "{}",
                caller.pane_id
            );
            assert_eq!(ledger, before);
        }
        assert_eq!(
            pull(&ledger, &recipient).unwrap().ids,
            vec![letter.id.clone()]
        );
        apply(&mut ledger, &recipient, None, &confirm, 2).unwrap();
        assert!(pull(&ledger, &recipient).unwrap().ids.is_empty());
    }
}
