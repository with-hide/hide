//! Prompt intake has a single total deadline. Pull is read-only; only a
//! successful stdout flush permits confirmation. All child processes are
//! owned and their bounded stdout is drained while they run.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hide_platform::fs::{self, private};
use hide_platform::process::{CaptureFailureKind, OwnedChild};
use serde::{Deserialize, Serialize};

const OUTPUT_LIMIT: usize = 64 * 1024;
const CONTEXT_LIMIT: usize = 8 * 1024;
const CONFIRM_RESERVE: Duration = Duration::from_millis(400);

#[derive(Deserialize)]
struct PromptInput {
    prompt: Option<String>,
    session_id: Option<String>,
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

pub fn read_prompt(payload: &[u8], truncated: bool) -> Prompt {
    let input = (!truncated)
        .then(|| serde_json::from_slice::<PromptInput>(payload).ok())
        .flatten();
    Prompt {
        digest: input
            .as_ref()
            .and_then(|input| input.prompt.as_deref())
            .map(prompt_digest),
        session: input
            .and_then(|input| input.session_id)
            .filter(|id| valid_session(id)),
    }
}

/// What the hook tells the core about the submitted prompt: the SHA-256 of
/// its trimmed text in lowercase hex. Only the core knows which line its
/// doorbell typed into the pane, so the prompt is the bell exactly when this
/// matches the digest of that line; the hook never decides it from the text,
/// and the prompt itself never leaves the hook.
pub fn prompt_digest(prompt: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(prompt.trim().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Whether `digest` is one [`prompt_digest`] could have made, so it may travel
/// as a `hide` argument.
pub fn valid_prompt_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The longest native session a letter pull or a Factory question guard
/// carries: Herdr takes an agent session of up to 4096 bytes, and Pi and omp
/// report theirs as a session file path, which on a deep checkout passes 250
/// bytes (PRD pi-omp-extension D-05).
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
    let mut arguments = vec!["inbox", "--hook"];
    if let Some(digest) = &prompt.digest {
        arguments.extend(["--prompt-digest", digest]);
    }
    if let Some(session) = &prompt.session {
        arguments.extend(["--session", session]);
    }
    let answer = run_cli(&arguments, pull_deadline)?;
    intake(answer)
}

/// The letters due while the agent works, asked at one of its tool events
/// for the native session `session` (PRD letters-to-busy-agents D-02): the
/// bodies a prompt's pull would hand over, once each letter's wait is over,
/// for a kind whose hook, plugin or extension takes them there. A resting
/// agent, a letter still waiting and a session that is not the pane's own
/// get nothing.
pub fn pull_at_tool(deadline: Instant, session: &str) -> Result<Option<Intake>, Failure> {
    let pull_deadline = deadline.checked_sub(CONFIRM_RESERVE).ok_or("deadline")?;
    let answer = run_cli(
        &["inbox", "--hook", "--session", session, "--tool"],
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
    if intake.ids.is_empty() && intake.context.is_empty() {
        Ok(None)
    } else {
        Ok(Some(intake))
    }
}

/// Whether `ids` may travel to `hide inbox --confirm` as its arguments: at
/// most one hook's five letters, each a letter id rather than an option.
pub fn valid_letter_ids(ids: &[String]) -> bool {
    ids.len() <= 5
        && ids.iter().all(|id| {
            !id.is_empty()
                && id.len() <= 256
                && !id.starts_with('-')
                && !id.chars().any(char::is_control)
        })
}

pub fn confirm(intake: &Intake, deadline: Instant) -> Result<(), Failure> {
    let mut arguments = vec!["inbox", "--confirm"];
    arguments.extend(intake.ids.iter().map(String::as_str));
    let answer = run_cli(&arguments, deadline)?;
    let confirmed = answer
        .get("confirmed")
        .and_then(serde_json::Value::as_array)
        .ok_or("confirm")?;
    if confirmed.len() != intake.ids.len()
        || !confirmed
            .iter()
            .zip(&intake.ids)
            .all(|(actual, expected)| actual.as_str() == Some(expected))
    {
        return Err("confirm".into());
    }
    Ok(())
}

fn run_cli(arguments: &[&str], deadline: Instant) -> Result<serde_json::Value, Failure> {
    run_cli_with_input(arguments, &[], deadline)
}

/// The `result` of one `hide` command run with `input` on its stdin, which
/// must fit a pipe's buffer: it is written whole before the answer is read.
pub(crate) fn run_cli_with_input(
    arguments: &[&str],
    input: &[u8],
    deadline: Instant,
) -> Result<serde_json::Value, Failure> {
    if Instant::now() >= deadline {
        return Err("deadline".into());
    }
    let executable = std::env::current_exe().map_err(|_| "cli")?;
    let sibling = executable
        .parent()
        .ok_or("cli")?
        .join(format!("hide{}", std::env::consts::EXE_SUFFIX));
    // Reuse the installed sibling rather than trusting a different checkout
    // earlier on PATH. A missing part follows the manual-inbox fallback.
    let mut command = Command::new(sibling);
    command
        .args(arguments)
        .stdin(if input.is_empty() {
            Stdio::null()
        } else {
            Stdio::piped()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = OwnedChild::spawn(&mut command).map_err(|_| "cli")?;
    if let Some(mut stdin) = child.take_stdin() {
        use std::io::Write as _;
        stdin.write_all(input).map_err(|_| "cli")?;
    }
    let output = child
        .capture_until(deadline, OUTPUT_LIMIT)
        .map_err(|failure| Failure {
            code: match failure.kind {
                CaptureFailureKind::Deadline => "deadline",
                CaptureFailureKind::OutputLimit { .. } => "format",
                CaptureFailureKind::Io(_) | CaptureFailureKind::Cleanup => "cli",
            },
            cleanup: failure.cleanup.as_ref().map(std::io::Error::kind),
        })?;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|_| "format")?;
    if !output.status.success() || value["ok"] != true {
        return Err(refusal_cause(value["reason"].as_str()).into());
    }
    value.get("result").cloned().ok_or_else(|| "format".into())
}

/// The cause a `hide` answer that is not `ok` is recorded under, from its
/// `reason`. A hook's request to its daemon names the step it stopped at
/// (`hided`'s `workspace_cli::request_hook`), and each step has a cause of
/// its own: no daemon published its socket (`unavailable`), the record
/// naming it is not trusted (`record`), nothing took the connection
/// (`connect`), the request was not written whole (`unsent`), no answer came
/// in time (`timeout`), the answer was unreadable (`answer`). A reason with
/// no cause of its own is the daemon's refusal (`refused`), and an answer
/// without one is malformed (`format`); `cli` is kept for a `hide` that could
/// not be run.
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
}

/// The fixed causes a hook may record: the delivery intake's fifteen (the
/// steps and refusals of [`refusal_cause`], `cli`, `deadline`, `format`,
/// `confirm` and `stdout`), the spawn guard's one (`guard`: the daemon could
/// not be asked), OpenCode's plugin's one (`plugin`: it skipped or gave up on
/// a helper call), Memory's three (`crate::memory`: the core could not be
/// asked, did not answer in time, or the thread asking panicked) and the
/// guardrails' three (`guardrails`: the policy file could not be read;
/// `guardrails_words`: a shell call named more words than are checked;
/// `guardrails_failed`: the hook failed or ran out of time before it judged a
/// call).
const CAUSES: [&str; 23] = [
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
    claim_at(home, cause, None)
}

/// [`claim`], keeping the `step` the failure stopped at beside a cause that
/// does not name it; a step outside [`CAUSES`] answers false.
pub(crate) fn claim_at(home: &Path, cause: &str, step: Option<&str>) -> bool {
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
            || state
                .last
                .keys()
                .chain(state.step.keys())
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

    #[test]
    fn only_a_complete_payload_carries_its_prompt_digest_and_session() {
        let payload = |prompt: &str| {
            serde_json::to_vec(&serde_json::json!({"session_id":"s","prompt":prompt})).unwrap()
        };
        let digest = |bytes: &[u8], truncated| read_prompt(bytes, truncated).digest;
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
            read_prompt(&payload("x"), false).session.as_deref(),
            Some("s")
        );
        assert_eq!(read_prompt(&payload("x"), true).session, None);
        assert_eq!(
            read_prompt(b"{\"session_id\":\"-rf\"}", false).session,
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
        assert!(claim_at(home.path(), "memory_unavailable", Some("connect")));
        let state: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(state["last"]["cli"], 0);
        assert!(state["last"]["memory_unavailable"].is_u64(), "{state}");
        assert_eq!(state["step"]["memory_unavailable"], "connect");
        assert!(!claim_at(
            home.path(),
            "memory_deadline",
            Some("not a cause")
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
