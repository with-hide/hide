//! Prompt intake has a single total deadline. Pull is read-only; only a
//! successful stdout flush permits confirmation. The pull and the
//! confirmation are hook requests the hook sends itself ([`ask`]); neither
//! starts a program (`crate::hook_request`).

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hide_platform::fs::{self, private};
use serde::{Deserialize, Serialize};

use crate::hook_request::{Caller, HookCommand, PullPoint, Timing};

const CONTEXT_LIMIT: usize = 8 * 1024;
const CONFIRM_RESERVE: Duration = Duration::from_millis(400);

#[derive(Deserialize)]
struct PromptInput {
    prompt: Option<String>,
    #[serde(flatten)]
    session: SessionInput,
}

#[derive(Deserialize)]
struct SessionInput {
    session_id: Option<String>,
    #[serde(rename = "sessionId")]
    camel_session_id: Option<String>,
}

impl SessionInput {
    fn resolve(self) -> Result<Option<String>, Failure> {
        // Some runtimes send both spellings. They must name one session;
        // choosing either side could hand another session its letters.
        let session = match (self.session_id, self.camel_session_id) {
            (Some(snake), Some(camel)) if snake != camel => return Err("session_conflict".into()),
            (Some(id), _) | (None, Some(id)) => Some(id),
            (None, None) => None,
        };
        Ok(session.filter(|id| valid_session(id)))
    }
}

/// The native identity used by prompt, tool and turn-end letter intake.
pub fn read_session(payload: &[u8], truncated: bool) -> Result<Option<String>, Failure> {
    if truncated {
        return Ok(None);
    }
    match serde_json::from_slice::<SessionInput>(payload) {
        Ok(input) => input.resolve(),
        Err(_) => Ok(None),
    }
}

/// The digest of the submitted prompt and the session id of a runtime's
/// `UserPromptSubmit` payload; a truncated or unreadable payload yields
/// neither, so its prompt is never taken for the bell and its hook never
/// counts as a submission.
#[derive(Default)]
pub struct Prompt {
    pub digest: Option<String>,
    pub session: Option<String>,
}

pub fn read_prompt(payload: &[u8], truncated: bool) -> Result<Prompt, Failure> {
    let input = (!truncated)
        .then(|| serde_json::from_slice::<PromptInput>(payload).ok())
        .flatten();
    let Some(input) = input else {
        return Ok(Prompt::default());
    };
    Ok(Prompt {
        digest: input.prompt.as_deref().map(prompt_digest),
        session: input.session.resolve()?,
    })
}

/// What the hook tells the core about the submitted prompt: the SHA-256 of
/// its canonical text ([`canonical_prompt`]) in lowercase hex. Every agent
/// and system uses this same identity rule, on both sides: the hook's prompt
/// and the session file's message, and the text Hide typed.
/// Only the core knows which line its
/// doorbell typed into the pane, so the prompt is the bell exactly when this
/// matches the digest of that line; the hook never decides it from the text,
/// and the prompt itself never leaves the hook.
pub fn prompt_digest(prompt: &str) -> String {
    full_prompt_digest(&canonical_prompt(prompt))
}

/// The text every input route sends. Expand tabs before the agent's editor
/// chooses its own tab stops, using the same four spaces as prompt identity.
/// Preserve every other character; the native record still decides intake.
pub fn input_text(text: &str) -> Cow<'_, str> {
    if text.contains('\t') {
        Cow::Owned(text.replace('\t', "    "))
    } else {
        Cow::Borrowed(text)
    }
}

/// What was typed, as each agent records it: Claude Code's record of a paste
/// reads as the text that was pasted (`hide_session::pasted::typed_text`),
/// line breaks (CR, LF, U+2028 and U+2029) and characters above U+FFFF are
/// removed, and a tab reads as four spaces, before trimming. A terminal paste
/// sends each line break as CR, an agent records LF, and Grok on Windows
/// takes the paste without them; a start's prompt argument carries its line
/// breaks as U+2028 and its tabs as four spaces
/// (`worktree_control::prompt_argument`), which Codex and agy record as
/// given and Claude Code with LF; some console readers drop characters
/// above U+FFFF, including the leading owl.
pub fn canonical_prompt(prompt: &str) -> String {
    let mut canonical = String::with_capacity(prompt.len());
    let typed = hide_session::pasted::typed_text(prompt);
    for c in input_text(&typed).chars() {
        match c {
            '\r' | '\n' | '\u{2028}' | '\u{2029}' => {}
            c if c > '\u{ffff}' => {}
            c => canonical.push(c),
        }
    }
    canonical.trim().to_owned()
}

/// Whether an agent's record of a person's message ends with `typed`: what
/// the composer already held when Hide typed stays in front of it, such as a
/// terminal report Antigravity on Windows took as typed (`27;1$y`, agent
/// journey, 2026-11-01). The record holds the whole typed text, so the agent
/// took it.
pub fn ends_with_typed(record: &str, typed: &str) -> bool {
    let typed = canonical_prompt(typed);
    !typed.is_empty() && canonical_prompt(record).ends_with(&typed)
}

/// Accept the canonical digest and the whole-line digest older hooks send
/// during an upgrade. Remove whole-line compatibility only when no hook
/// older than this build can still run. A saved old bell becomes ineligible
/// at its letter's delivery deadline, so the ledger needs no migration.
pub fn matches_prompt_digest(line: &str, digest: &str) -> bool {
    prompt_digest(line) == digest || full_prompt_digest(line) == digest
}

fn full_prompt_digest(prompt: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(prompt.trim().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Whether `digest` is one [`prompt_digest`] could have made, so it may travel
/// as a `hide` argument.
pub fn valid_prompt_digest(digest: &str) -> bool {
    valid_sha256_digest(digest)
}

/// A lowercase SHA-256 digest, whether of a prompt or a native session.
pub fn valid_sha256_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The longest native session a letter pull carries: the engine takes an
/// agent session of up to 4096 bytes, and Pi and omp report theirs as a
/// session file path, which on a deep checkout passes 250 bytes (PRD
/// pi-omp-extension D-05).
pub const SESSION_LIMIT: usize = 4096;

/// Whether a session id or session file path may travel as a `hide` argument.
pub fn valid_session(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= SESSION_LIMIT
        && !id.starts_with('-')
        && !id.chars().any(char::is_control)
}

#[derive(Deserialize)]
pub struct Intake {
    pub context: String,
    pub ids: Vec<String>,
    /// The pane's plan summary a prompt's pull carries (PRD hide-plan
    /// D-30); absent from a tool event's pull and from an older daemon.
    /// Read only once the letters are taken, so one this helper cannot read
    /// costs the summary, never the letters beside it: a device's helper may
    /// be older than the core it asks.
    #[serde(default)]
    pub plan_context: Option<serde_json::Value>,
}

#[derive(Debug)]
pub struct Failure {
    pub code: &'static str,
    pub cleanup: Option<std::io::ErrorKind>,
}

impl From<&'static str> for Failure {
    fn from(code: &'static str) -> Self {
        Self {
            code,
            cleanup: None,
        }
    }
}

/// What the agent receives for the submitted prompt: the letter bodies when
/// the core recognizes the prompt as the line its doorbell typed, otherwise
/// at most a count.
pub fn pull(deadline: Instant, prompt: &Prompt) -> Result<Option<Intake>, Failure> {
    let pull_deadline = deadline.checked_sub(CONFIRM_RESERVE).ok_or("deadline")?;
    let answer = ask(
        HookCommand::LetterPull {
            prompt_digest: prompt.digest.clone(),
            session: prompt.session.clone(),
            point: PullPoint::Prompt,
            bundle: true,
        },
        pull_deadline,
    )?;
    let intake = intake(answer)?;
    Ok(intake.and_then(|intake| keep_plan(intake, prompt.session.as_deref())))
}

/// Keeps the pull's plan summary in the pane's record for the pre-tool
/// hook, and adds its line to what the agent reads (PRD hide-plan D-14,
/// D-30). Outside a pane Hide manages, or without the session, nothing is kept.
fn keep_plan(mut intake: Intake, session: Option<&str>) -> Option<Intake> {
    let raw = intake.plan_context.take();
    let plan = raw.and_then(|raw| {
        serde_json::from_value::<crate::plan_record::PlanContext>(raw)
            .inspect_err(|_| {
                if let Ok(home) = hide_platform::host::home_dir() {
                    diagnose(&home, "plan_context_format");
                }
            })
            .ok()
    });
    if let Some(plan) = plan {
        let pane = crate::pane::from_env();
        if let (Some(pane), Some(session), Ok(home)) =
            (pane, session, hide_platform::host::home_dir())
            && crate::plan_record::write(&home, &pane, session, &plan).is_err()
        {
            diagnose(&home, "plan_record");
        }
        if let Some(hint) = crate::plan_record::hint(&plan, crate::hook_log::now_ms()) {
            if !intake.context.is_empty() && !intake.context.ends_with('\n') {
                intake.context.push('\n');
            }
            intake.context.push_str(&hint);
        }
    }
    (!intake.ids.is_empty() || !intake.context.is_empty()).then_some(intake)
}

/// The letters due while the agent works, asked at one of its tool events
/// for the native session `session` (PRD letters-to-busy-agents D-02): the
/// bodies a prompt's pull would hand over, once each letter's wait is over,
/// for a kind whose hook, plugin or extension takes them there. A resting
/// agent, a letter still waiting and a session that is not the pane's own
/// get nothing.
pub fn pull_at_tool(deadline: Instant, session: &str) -> Result<Option<Intake>, Failure> {
    pull_at_gap(deadline, session, PullPoint::Tool)
}

pub fn pull_at_turn_end(deadline: Instant, session: &str) -> Result<Option<Intake>, Failure> {
    pull_at_gap(deadline, session, PullPoint::TurnEnd)
}

fn pull_at_gap(
    deadline: Instant,
    session: &str,
    point: PullPoint,
) -> Result<Option<Intake>, Failure> {
    let pull_deadline = deadline.checked_sub(CONFIRM_RESERVE).ok_or("deadline")?;
    let answer = ask(
        HookCommand::LetterPull {
            prompt_digest: None,
            session: Some(session.to_owned()),
            point,
            bundle: true,
        },
        pull_deadline,
    )?;
    intake(answer)
}

fn intake(answer: serde_json::Value) -> Result<Option<Intake>, Failure> {
    let intake: Intake = serde_json::from_value(answer).map_err(|_| "format")?;
    if intake.context.len() > CONTEXT_LIMIT || !valid_letter_ids(&intake.ids) {
        return Err("format".into());
    }
    // A count-only answer carries a context line and nothing to confirm.
    if intake.ids.is_empty() && intake.context.is_empty() && intake.plan_context.is_none() {
        Ok(None)
    } else {
        Ok(Some(intake))
    }
}

/// The most letters one handover bundle carries, the letters it shows in
/// full and those it folds, all confirmed together; the core's open-letter
/// limit keeps a bundle under it.
pub const BUNDLE_LETTERS: usize = 1_024;

/// Whether `ids` may be confirmed in one request, a hook's or `hide inbox
/// --confirm`'s: at most one bundle's letters, each a letter id rather than
/// an option.
pub fn valid_letter_ids(ids: &[String]) -> bool {
    ids.len() <= BUNDLE_LETTERS
        && ids.iter().all(|id| {
            !id.is_empty()
                && id.len() <= 256
                && !id.starts_with('-')
                && !id.chars().any(char::is_control)
        })
}

/// Tells the daemon the letters `intake` handed over reached the agent, in
/// as many requests as its ids take ([`confirm_runs`]), each answered with
/// exactly the ids it named.
pub fn confirm(intake: &Intake, deadline: Instant) -> Result<(), Failure> {
    for ids in confirm_runs(&intake.ids) {
        let answer = ask(HookCommand::LetterConfirm { ids: ids.to_vec() }, deadline)?;
        let confirmed = answer
            .get("confirmed")
            .and_then(serde_json::Value::as_array)
            .ok_or("confirm")?;
        if confirmed.len() != ids.len()
            || !confirmed
                .iter()
                .zip(ids)
                .all(|(actual, expected)| actual.as_str() == Some(expected))
        {
            return Err("confirm".into());
        }
    }
    Ok(())
}

/// The most bytes one confirmation's ids take, encoded: half a hook
/// request, so the request's envelope (pane, nonce, installation, proof)
/// fits beside them.
const CONFIRM_IDS_BYTES: usize = crate::hook_request::HOOK_REQUEST_LIMIT / 2;

/// `ids` cut, in order, into runs whose encoded ids fit one confirmation: a
/// bundle of up to [`BUNDLE_LETTERS`] letters can name more than one hook
/// request carries, and a bundle that could never be confirmed would be
/// handed over again at every prompt.
fn confirm_runs(ids: &[String]) -> Vec<&[String]> {
    let mut runs = Vec::new();
    let (mut start, mut bytes) = (0, 0);
    for (at, id) in ids.iter().enumerate() {
        // The id quoted and escaped, and its comma.
        let size = serde_json::to_string(id).map_or(id.len() * 6 + 2, |text| text.len()) + 1;
        if at > start && bytes + size > CONFIRM_IDS_BYTES {
            runs.push(&ids[start..at]);
            (start, bytes) = (at, 0);
        }
        bytes += size;
    }
    if start < ids.len() {
        runs.push(&ids[start..]);
    }
    runs
}

/// The `result` of one hook request this hook sends the daemon itself
/// ([`crate::hook_request`]), never through a `hide` it starts.
pub fn ask(command: HookCommand, deadline: Instant) -> Result<serde_json::Value, Failure> {
    ask_timed(command, deadline).0
}

/// [`ask`], with how long its steps took.
pub fn ask_timed(
    command: HookCommand,
    deadline: Instant,
) -> (Result<serde_json::Value, Failure>, Timing) {
    let (answer, timing) = ask_answer(command, deadline);
    let result = answer.and_then(|mut answer| {
        answer
            .get_mut("result")
            .map(serde_json::Value::take)
            .ok_or_else(|| "format".into())
    });
    (result, timing)
}

/// The daemon's whole `ok` answer to one hook request, `result` and all,
/// with how long its steps took; an answer that is not `ok` is its cause.
pub fn ask_answer(
    command: HookCommand,
    deadline: Instant,
) -> (Result<serde_json::Value, Failure>, Timing) {
    let (answer, timing) = answer_of(command, deadline);
    let answer = answer.and_then(|value| {
        if value["ok"] != true {
            return Err(refusal_cause(value["reason"].as_str()).into());
        }
        Ok(value)
    });
    (answer, timing)
}

/// The daemon's answer to one hook request, `ok` or a refusal with its
/// `reason`; only a request no daemon answered is the cause it stopped at.
pub fn answer_of(
    command: HookCommand,
    deadline: Instant,
) -> (Result<serde_json::Value, Failure>, Timing) {
    if Instant::now() >= deadline {
        return (Err("deadline".into()), Timing::default());
    }
    let home = match hide_platform::host::home_dir() {
        Ok(home) => home,
        Err(_) => return (Err("unavailable".into()), Timing::default()),
    };
    let state_dir = match crate::route::state_dir(&home) {
        Ok(state_dir) => state_dir,
        Err(cause) => return (Err(cause.into()), Timing::default()),
    };
    let pane_id = crate::pane::from_env();
    let install = crate::route::pane_install();
    let named = std::env::var_os(CAP_REF_VARIABLE).map(PathBuf::from);
    let caller = Caller {
        state_dir: &state_dir,
        pane_id: pane_id.as_deref(),
        install: install.as_deref(),
        named: named.as_deref(),
    };
    let request_id = match crate::hook_request::fresh_request_id() {
        Ok(request_id) => request_id,
        Err(_) => return (Err("cli".into()), Timing::default()),
    };
    let sent = crate::hook_request::send(&caller, &request_id, command, deadline);
    let answer = sent
        .answer
        .map_err(|failure| Failure::from(refusal_cause(Some(failure.reason))));
    (answer, sent.timing)
}

/// The variable a pane names its persistent Workspace reference by, which a
/// hook request names for its fresh proof to agree with.
const CAP_REF_VARIABLE: &str = "HIDE_CAP_REF";

/// The cause a `hide` answer that is not `ok` is recorded under, from its
/// `reason`. A hook's request to its daemon names the step it stopped at
/// (`hided`'s `workspace_cli::request_hook`), and each step has a cause of
/// its own: no daemon published its socket (`unavailable`), the record
/// naming it is not trusted (`record`), nothing took the connection
/// (`connect`), the request was not written whole (`unsent`), no answer came
/// in time (`timeout`), the answer was unreadable (`answer`). The pane's own
/// installation did not answer: only another one did, refusing the pane
/// (`other_install`), or one answered without naming the pane's, as an
/// earlier build does (`unconfirmed`; `hook_request::install`). A reason with
/// no cause of its own is the daemon's refusal (`refused`), and an answer
/// without one is malformed (`format`); `cli` is a request the hook could not
/// form.
pub fn refusal_cause(reason: Option<&str>) -> &'static str {
    match reason {
        Some("ledger_unavailable") => "ledger",
        Some("capacity") => "capacity",
        Some("agent_pane_required" | "caller_identity_conflict" | "caller_context_changed") => {
            "identity"
        }
        Some("hide_unavailable") => "unavailable",
        Some("invalid_bootstrap_socket_record") => "record",
        Some("hide_unreachable") => "connect",
        Some("request_unsent") => "unsent",
        Some("request_timeout") => "timeout",
        Some("invalid_response") => "answer",
        Some(crate::hook_request::install::INSTALL_UNCONFIRMED) => "unconfirmed",
        Some(crate::hook_request::install::OTHER_INSTALL) => "other_install",
        Some(_) => "refused",
        None => "format",
    }
}

#[derive(Deserialize, Serialize, Default)]
struct Diagnostics {
    last: BTreeMap<String, u64>,
    /// The step a cause that does not name one itself last stopped at
    /// (Memory's `memory_unavailable`), one of [`CAUSES`]. A file written
    /// before steps were kept has none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    step: BTreeMap<String, String>,
    /// How long the stages of the request a cause last recorded took
    /// (Memory's), so the record says which stage the time went to. A file
    /// written before stages were kept has none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    stages: BTreeMap<String, Stages>,
}

/// When each stage of one hook request ended, in milliseconds from the start
/// of its budget: the request began (the hook's input read), the stream to
/// the daemon was connected, the answer was in, and the wait ended; a stage
/// the request did not reach is absent.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct Stages {
    pub asked_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_ms: Option<u64>,
    pub ended_ms: u64,
}

/// The fixed causes a hook may record: the delivery intake's eighteen (the
/// steps and refusals of [`refusal_cause`], `cli`, `deadline`, `format`,
/// `confirm`, `stdout` and `session_conflict`), the guards' one (`guard`: the daemon could
/// not be asked), OpenCode's plugin's one (`plugin`: it skipped or gave up on
/// a helper call), Memory's three (`crate::memory`: the core could not be
/// asked, did not answer in time, or the thread asking panicked) and the
/// guardrails' three (`guardrails`: the policy file could not be read;
/// `guardrails_words`: a shell call named more words than are checked;
/// `guardrails_failed`: the hook failed or ran out of time before it judged a
/// call).
const CAUSES: [&str; 28] = [
    "session_conflict",
    "cli",
    "deadline",
    "format",
    "confirm",
    "ledger",
    "capacity",
    "identity",
    "unavailable",
    "record",
    "connect",
    "unsent",
    "timeout",
    "answer",
    "unconfirmed",
    "other_install",
    "refused",
    "stdout",
    "guard",
    "plugin",
    "memory_unavailable",
    "memory_deadline",
    "memory_panicked",
    "guardrails",
    "guardrails_words",
    "guardrails_failed",
    "plan_record",
    "plan_context_format",
];

/// Fixed causes ([`CAUSES`]), private storage and a nonblocking cross-process lock.
/// A hook with no pane can still record its failure without recording its cwd.
pub fn diagnose(home: &Path, cause: &'static str) -> bool {
    diagnose_failure(home, &cause.into())
}

pub fn diagnose_failure(home: &Path, failure: &Failure) -> bool {
    let cause = failure.code;
    // Emit only after the cross-process claim is persisted. Damaged or
    // unavailable throttle storage cannot authorize an unthrottled log.
    if claim(home, cause) {
        eprintln!(
            "{}",
            serde_json::json!({"component":"delivery_hook","kind":"intake.failed","code":cause,"cleanup_io_kind":failure.cleanup.map(|kind| format!("{kind:?}")),"diagnostic_saved":true})
        );
        true
    } else {
        false
    }
}

/// Claims the right to record `cause` once per ten minutes, across processes.
/// True only when this call persisted the claim; a cause outside [`CAUSES`], a
/// busy lock and damaged or unavailable storage all answer false.
pub(crate) fn claim(home: &Path, cause: &str) -> bool {
    claim_at(home, cause, None, None)
}

/// [`claim`], keeping the `step` the failure stopped at beside a cause that
/// does not name it, and the request's `stages`; a step outside [`CAUSES`]
/// answers false.
pub(crate) fn claim_at(
    home: &Path,
    cause: &str,
    step: Option<&str>,
    stages: Option<Stages>,
) -> bool {
    if !CAUSES.contains(&cause) || step.is_some_and(|step| !CAUSES.contains(&step)) {
        return false;
    }
    let path = diagnostic_path(home);
    let result = (|| -> std::io::Result<bool> {
        let parent = path
            .parent()
            .ok_or_else(|| std::io::Error::other("diagnostic parent unavailable"))?;
        private::create_dir_all(parent)?;
        let file = private::open_own_file(&parent.join("delivery-diagnostics.lock"), true)?;
        let fs::lock::Waited::Locked(_lock) =
            fs::lock::lock_file(file, fs::lock::Mode::Exclusive, Duration::ZERO, &|| false)?
        else {
            return Ok(false);
        };
        let mut state = match private::open_own_file(&path, false) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(2049).read_to_end(&mut bytes)?;
                if bytes.len() > 2048 {
                    return Err(std::io::Error::other("diagnostic capacity"));
                }
                serde_json::from_slice::<Diagnostics>(&bytes).map_err(std::io::Error::other)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Diagnostics::default(),
            Err(error) => return Err(error),
        };
        if state.last.len() > CAUSES.len()
            || state.step.len() > CAUSES.len()
            || state.stages.len() > CAUSES.len()
            || state
                .last
                .keys()
                .chain(state.step.keys())
                .chain(state.stages.keys())
                .chain(state.step.values())
                .any(|cause| !CAUSES.contains(&cause.as_str()))
        {
            return Err(std::io::Error::other("diagnostic capacity"));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(std::io::Error::other)?
            .as_millis() as u64;
        if state
            .last
            .get(cause)
            .is_some_and(|last| now.saturating_sub(*last) < 10 * 60_000)
        {
            return Ok(false);
        }
        if state.last.len() >= CAUSES.len() && !state.last.contains_key(cause) {
            return Err(std::io::Error::other("diagnostic capacity"));
        }
        state.last.insert(cause.to_owned(), now);
        if let Some(step) = step {
            state.step.insert(cause.to_owned(), step.to_owned());
        }
        if let Some(stages) = stages {
            state.stages.insert(cause.to_owned(), stages);
        }
        let bytes = serde_json::to_vec(&state).map_err(std::io::Error::other)?;
        fs::atomic::write_file(&path, &bytes, fs::Access::Private)?;
        Ok(true)
    })();
    matches!(result, Ok(true))
}

fn diagnostic_path(home: &Path) -> PathBuf {
    crate::counters::state_directory(home)
        .parent()
        .expect("counter directory has a parent")
        .join("delivery-diagnostics.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The largest bundle, of the longest ids a confirmation takes, is
    /// confirmed in order and whole, in runs that each fit one hook request
    /// beside its envelope; the bundles a prompt hands over in practice take
    /// one.
    #[test]
    fn a_bundle_is_confirmed_in_runs_that_each_fit_one_request() {
        let longest: Vec<String> = (0..BUNDLE_LETTERS)
            .map(|n| format!("{n:0>256}"))
            .collect();
        assert!(valid_letter_ids(&longest));
        let usual: Vec<String> = (28_000..28_000 + BUNDLE_LETTERS)
            .map(|n| format!("letter-{n}"))
            .collect();
        for ids in [&longest, &usual] {
            let runs = confirm_runs(ids);
            assert_eq!(runs.concat(), *ids);
            for run in &runs {
                assert!(!run.is_empty());
                let command = serde_json::to_string(&HookCommand::LetterConfirm { ids: run.to_vec() })
                    .unwrap();
                assert!(command.len() <= CONFIRM_IDS_BYTES + 64, "{}", command.len());
            }
        }
        assert_eq!(confirm_runs(&usual[..20]).len(), 1);
        assert!(confirm_runs(&[]).is_empty());
    }

    #[test]
    fn a_plan_summary_this_helper_cannot_read_leaves_the_letters_to_deliver() {
        let answer = serde_json::json!({
            "context": "letter body",
            "ids": ["letter-1"],
            "plan_context": {"plan": null, "mission": null, "conductor": null, "later": 1},
        });
        let intake = intake(answer).unwrap().unwrap();
        assert_eq!(
            (intake.context.as_str(), intake.ids.as_slice()),
            ("letter body", &["letter-1".to_owned()][..])
        );
        assert!(
            serde_json::from_value::<crate::plan_record::PlanContext>(intake.plan_context.unwrap())
                .is_err()
        );
    }

    #[test]
    fn prompt_identity_ignores_only_non_bmp_characters_and_outer_whitespace() {
        // FIPS 180-2's SHA-256 of "abc", independent of the bell producer.
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        for prompt in ["abc", "🦉 abc", "🔔 a🐈bc 🦉", "\u{10000}abc\u{10ffff}"] {
            assert_eq!(prompt_digest(prompt), abc, "{prompt:?}");
        }
        for prompt in ["abd", "a bc", "Abc", "abc\u{ffff}", "abc\u{fe0f}", "abc한"] {
            assert_ne!(prompt_digest(prompt), abc, "{prompt:?}");
        }
    }

    /// A start's prompt argument carries line breaks as U+2028 and a tab as
    /// four spaces; Codex records it as given and Claude Code with LF, and
    /// each record is the brief Hide launched.
    #[test]
    fn a_launched_brief_with_a_tab_is_recognized_in_each_agents_record() {
        let brief = "# Brief\n\tstep one\nReport by letter.";
        // What the start carried, as `worktree_control::prompt_argument`
        // writes it.
        let argument = "# Brief\u{2028}    step one\u{2028}Report by letter.";
        let claude = "# Brief\n    step one\nReport by letter.";
        for record in [argument, claude] {
            assert!(
                matches_prompt_digest(brief, &prompt_digest(record)),
                "{record:?}"
            );
            assert!(
                ends_with_typed(&format!("27;1$y{record}"), brief),
                "{record:?}"
            );
        }
        assert!(!matches_prompt_digest(
            brief,
            &prompt_digest("# Brief\n  step one\nReport by letter.")
        ));
        // A CLI's three-space tab expansion is not the text Hide sent.
        assert!(!matches_prompt_digest(
            brief,
            &prompt_digest("# Brief\n   step one\nReport by letter.")
        ));
    }

    #[test]
    fn every_route_expands_tabs_before_an_agent_can_choose_its_tab_width() {
        let brief = "@journey-missing-file.md names no file of this repository.\n\tIt needs no change.\n\nReply with exactly TOKEN";
        let expected = "@journey-missing-file.md names no file of this repository.\n    It needs no change.\n\nReply with exactly TOKEN";
        assert_eq!(input_text(brief), expected);
        assert_eq!(prompt_digest(expected), prompt_digest(brief));
        assert!(matches!(input_text(expected), Cow::Borrowed(_)));
    }

    #[test]
    fn a_brief_claude_code_wrapped_as_a_paste_is_the_brief_hide_typed() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/claude-pasted-brief-prompt.json"
        ))
        .unwrap();
        let typed = fixture["typed"].as_str().unwrap();
        let payload = serde_json::to_vec(&fixture["payload"]).unwrap();
        let reported = read_prompt(&payload, false).unwrap().digest.unwrap();
        // The SHA-256 of the brief's own trimmed text, computed apart from
        // the canonical rule under test.
        use sha2::{Digest, Sha256};
        let expected: String = Sha256::digest(typed.replace('\n', "").trim().as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(reported, expected);
        assert!(matches_prompt_digest(typed, &reported));
        // A terminal paste sends each line break as CR, the agent records LF,
        // and Grok on Windows records the paste without them (agent journey,
        // 2026-11-01): the line breaks are not what identifies it.
        assert!(matches_prompt_digest(&typed.replace('\n', "\r"), &reported));
        assert!(matches_prompt_digest(
            &typed.replace('\n', "\r\n"),
            &reported
        ));
        assert!(matches_prompt_digest(&typed.replace('\n', ""), &reported));
        // Typed words beside a paste are not the paste.
        let prompt = fixture["payload"]["prompt"].as_str().unwrap();
        assert_ne!(prompt_digest(&format!("also {prompt}")), expected);
    }

    #[test]
    fn the_prompt_hook_and_journey_agree_on_the_shared_identity_examples() {
        let examples: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/bell-prompt-identity.json"))
                .unwrap();
        let saved = examples["bell_line"].as_str().unwrap();
        let expected_digest = examples["digest"].as_str().unwrap();
        assert_eq!(prompt_digest(saved), expected_digest);
        for example in examples["cases"].as_array().unwrap() {
            assert_eq!(
                matches_prompt_digest(saved, &prompt_digest(example["prompt"].as_str().unwrap())),
                example["matches"].as_bool().unwrap(),
                "{}",
                example["name"]
            );
        }
    }

    #[test]
    fn only_a_complete_payload_carries_its_prompt_digest_and_session() {
        let payload = |prompt: &str| {
            serde_json::to_vec(&serde_json::json!({"session_id":"s","prompt":prompt})).unwrap()
        };
        let digest = |bytes: &[u8], truncated| read_prompt(bytes, truncated).unwrap().digest;
        // The SHA-256 of "abc", from FIPS 180-2's first example.
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(digest(&payload("abc"), false).as_deref(), Some(abc));
        assert_eq!(digest(&payload(" abc\n"), false).as_deref(), Some(abc));
        assert_eq!(digest(&payload("abc"), true), None);
        assert_ne!(digest(&payload("abc d"), false).as_deref(), Some(abc));
        assert_eq!(digest(b"{\"session_id\":\"s\"}", false), None);
        assert_eq!(digest(b"not json", false), None);
        assert_eq!(digest(b"", false), None);
        assert!(valid_prompt_digest(abc));
        assert!(!valid_prompt_digest(&abc.to_uppercase()));
        assert!(!valid_prompt_digest(&abc[1..]));
        assert_eq!(
            read_prompt(&payload("x"), false)
                .unwrap()
                .session
                .as_deref(),
            Some("s")
        );
        assert_eq!(read_prompt(&payload("x"), true).unwrap().session, None);
        for input in [
            br#"{"sessionId":"s","prompt":"abc"}"#.as_slice(),
            br#"{"sessionId":"s","session_id":"s","prompt":"abc"}"#.as_slice(),
        ] {
            let prompt = read_prompt(input, false).unwrap();
            assert_eq!(prompt.session.as_deref(), Some("s"));
            assert_eq!(prompt.digest.as_deref(), Some(abc));
        }
        assert_eq!(
            read_prompt(br#"{"sessionId":"child","session_id":"parent"}"#, false)
                .err()
                .unwrap()
                .code,
            "session_conflict"
        );
        assert_eq!(
            read_prompt(b"{\"session_id\":\"-rf\"}", false)
                .unwrap()
                .session,
            None
        );
    }

    #[test]
    fn diagnostic_claim_precedes_output_and_repeated_cause_is_suppressed() {
        let home = tempfile::tempdir().unwrap();
        assert!(diagnose(home.path(), "cli"));
        assert!(!diagnose(home.path(), "cli"));
        assert!(diagnose(home.path(), "deadline"));
        let stored = std::fs::read(diagnostic_path(home.path())).unwrap();
        let state: Diagnostics = serde_json::from_slice(&stored).unwrap();
        assert_eq!(state.last.len(), 2);
        assert!(hide_platform::fs::private::is_private(&diagnostic_path(home.path())).unwrap());
        assert!(
            !String::from_utf8(stored)
                .unwrap()
                .contains(&home.path().display().to_string())
        );
    }

    /// Each step of a hook's request (`hided`'s `request_hook`) is recorded
    /// under a cause of its own, and every cause a refusal can have is one a
    /// hook may record, so no refusal goes unrecorded.
    #[test]
    fn each_step_of_a_hook_request_is_recorded_under_its_own_cause() {
        let steps = [
            ("hide_unavailable", "unavailable"),
            ("invalid_bootstrap_socket_record", "record"),
            ("hide_unreachable", "connect"),
            ("request_unsent", "unsent"),
            ("request_timeout", "timeout"),
            ("invalid_response", "answer"),
            ("install_unconfirmed", "unconfirmed"),
            ("other_install", "other_install"),
        ];
        for (reason, cause) in steps {
            assert_eq!(refusal_cause(Some(reason)), cause, "{reason}");
        }
        for (reason, cause) in [
            (Some("ledger_unavailable"), "ledger"),
            (Some("capacity"), "capacity"),
            (Some("caller_identity_conflict"), "identity"),
            (Some("pane_unavailable"), "refused"),
            (None, "format"),
        ] {
            assert_eq!(refusal_cause(reason), cause, "{reason:?}");
        }
        let home = tempfile::tempdir().unwrap();
        for (reason, _) in steps {
            assert!(
                diagnose(home.path(), refusal_cause(Some(reason))),
                "{reason}"
            );
        }
        assert!(diagnose(
            home.path(),
            refusal_cause(Some("pane_unavailable"))
        ));
    }

    /// A file an earlier helper wrote, with no steps, is read and kept; a
    /// cause that does not name its step keeps the one it stopped at.
    #[test]
    fn a_step_is_kept_beside_its_cause_and_an_earlier_file_still_reads() {
        let home = tempfile::tempdir().unwrap();
        let path = diagnostic_path(home.path());
        private::create_dir_all(path.parent().unwrap()).unwrap();
        fs::atomic::write_file(&path, br#"{"last":{"cli":0}}"#, fs::Access::Private).unwrap();
        assert!(claim_at(
            home.path(),
            "memory_unavailable",
            Some("connect"),
            None
        ));
        let state: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(state["last"]["cli"], 0);
        assert!(state["last"]["memory_unavailable"].is_u64(), "{state}");
        assert_eq!(state["step"]["memory_unavailable"], "connect");
        assert!(!claim_at(
            home.path(),
            "memory_deadline",
            Some("not a cause"),
            None
        ));
    }

    #[test]
    fn corrupt_or_unavailable_diagnostic_storage_never_bypasses_throttle() {
        for damaged in [
            b"damaged private bytes".as_slice(),
            b"{\"last\":{\"unknown\":0}}".as_slice(),
            b"{\"last\":{},\"step\":{\"memory_unavailable\":\"unknown\"}}".as_slice(),
        ] {
            let home = tempfile::tempdir().unwrap();
            let path = diagnostic_path(home.path());
            private::create_dir_all(path.parent().unwrap()).unwrap();
            fs::atomic::write_file(&path, damaged, fs::Access::Private).unwrap();
            for _ in 0..3 {
                assert!(!diagnose(home.path(), "cli"));
            }
            assert_eq!(std::fs::read(&path).unwrap(), damaged);
        }
        let home = tempfile::tempdir().unwrap();
        let path = diagnostic_path(home.path());
        private::create_dir_all(path.parent().unwrap()).unwrap();
        // A real invalid store shape fails before an output claim.
        private::create_dir_all(&path).unwrap();
        assert!(!diagnose(home.path(), "cli"));
        assert!(!diagnose(home.path(), "unexpected"));
        assert!(path.is_dir());
    }
}
