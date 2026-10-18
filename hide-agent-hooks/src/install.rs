//! Reading, appending to, and removing from a runtime's global hook file.
//!
//! Every rule here exists because the array Hide appends to is already
//! occupied. On the machine this was designed against, `SubagentStart`
//! already carried two other tools' hooks (PRD D-26), so:
//!
//! - a file that does not parse is a file Hide never writes (PRD D-46);
//! - an append never rewrites an entry it does not own;
//! - a removal takes only the entries carrying Hide's marker (PRD D-57);
//! - installing twice converges instead of duplicating (PRD B25).

use std::fs;
use std::io::{ErrorKind, Read};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::route::HookTarget;
use crate::runtime::{AgentRuntime, HookEvent, hook_source_id, marker_version_in};

/// Why Hide did not change a runtime's hook file.
///
/// Each variant names a cause the operator can act on; none of them is a
/// state Hide papers over by writing anyway.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum InstallFailure {
    /// The file exists but could not be read: permissions, or an I/O error.
    Unreadable { path: String, detail: String },
    /// The file is not valid JSON. Hide cannot append to what it cannot
    /// parse, and rewriting it from scratch would delete another tool's
    /// hooks, so it stops here (PRD D-46).
    Unparsable { path: String, detail: String },
    /// The file parses but `hooks`, or one event under it, is not the shape
    /// the runtime documents.
    UnexpectedShape { path: String, detail: String },
    /// The write itself failed.
    NotWritable { path: String, detail: String },
    /// The file says no to the change: what it holds is the operator's
    /// decision, which Hide does not override.
    Refused { path: String, detail: String },
    /// Hide's entry points at a helper that is no longer on disk, so the hook
    /// runs nothing. Reinstalling from the running app repairs it.
    HelperMissing { path: String, helper: String },
}

impl InstallFailure {
    /// One sentence for the operator, in the Settings diagnosis and the CLI.
    pub fn message(&self) -> String {
        match self {
            Self::Unreadable { path, detail } => format!("{path} could not be read: {detail}"),
            Self::Unparsable { path, detail } => {
                format!("{path} is not valid JSON ({detail}); Hide left it untouched")
            }
            Self::UnexpectedShape { path, detail } => {
                format!("{path} has an unexpected shape: {detail}")
            }
            Self::NotWritable { path, detail } => format!("{path} could not be written: {detail}"),
            Self::Refused { path, detail } => format!("{detail} ({path})"),
            Self::HelperMissing { helper, .. } => {
                format!("the installed hook points at {helper}, which is missing")
            }
        }
    }
}

/// What Hide currently finds in one runtime's hook file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum HookStatus {
    /// The runtime is not set up on this Mac, so there is nothing to install.
    RuntimeAbsent,
    /// Hide's entries are present at the current version.
    Installed { version: u32 },
    /// Hide's entries are present at an older version, or are missing from
    /// some of the events Hide registers, or select other tools than the
    /// ones Hide writes today.
    Outdated { version: u32 },
    /// The runtime is here and carries no entry of Hide's.
    NotInstalled,
    /// The operator switched the agent off in Settings, Agents, so Hide's
    /// hook is meant to be absent. Nothing in the runtime's file says so:
    /// the kit's record does, and whoever holds it sets this.
    Off,
    /// The file could not be read, parsed or written.
    Failed { reason: InstallFailure },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallOutcome {
    /// False when the file already said exactly this; the second install of
    /// the same intent writes nothing (engineering rule 11).
    pub changed: bool,
    /// Entries belonging to other tools that survived the append. The count is
    /// what the regression test asserts.
    pub preserved_entries: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoveOutcome {
    pub changed: bool,
    pub removed_entries: usize,
    pub preserved_entries: usize,
}

/// The `hooks` object every runtime keeps at the top level of its file.
const HOOKS_KEY: &str = "hooks";

/// Judges one runtime without changing anything.
pub fn status(runtime: AgentRuntime, home: &Path) -> HookStatus {
    if !runtime.home_directory(home).is_dir() {
        return HookStatus::RuntimeAbsent;
    }
    let path = runtime.config_path(home);
    let document = match read_document(&path) {
        Ok(Some(document)) => document,
        Ok(None) => return HookStatus::NotInstalled,
        Err(reason) => return HookStatus::Failed { reason },
    };
    let hooks = match hooks_object(&document, &path) {
        Ok(Some(hooks)) => hooks,
        Ok(None) => return HookStatus::NotInstalled,
        Err(reason) => return HookStatus::Failed { reason },
    };
    let mut lowest: Option<u32> = None;
    let mut missing_event = false;
    for event in HookEvent::ALL {
        let groups = hooks.get(event.name()).and_then(Value::as_array);
        // An entry of Hide's that selects other tools than the ones Hide
        // writes today is outdated whatever its version says: the file does
        // not hold what an install would write.
        missing_event |= groups.is_some_and(|groups| {
            groups.iter().any(|group| {
                group_marker_version(group).is_some()
                    && group.get("matcher").and_then(Value::as_str) != hook_matcher(runtime, event)
            })
        });
        let version = groups
            .map(|groups| owned_versions(groups))
            .and_then(|versions| versions.into_iter().min());
        match version {
            Some(version) => lowest = Some(lowest.map_or(version, |best| best.min(version))),
            None => missing_event = true,
        }
    }
    let Some(version) = lowest else {
        return HookStatus::NotInstalled;
    };
    if let Some(helper) = installed_helper(hooks)
        && !Path::new(&helper).exists()
    {
        return HookStatus::Failed {
            reason: InstallFailure::HelperMissing {
                path: path.display().to_string(),
                helper,
            },
        };
    }
    if missing_event || version < crate::runtime::HOOK_ENTRY_VERSION {
        HookStatus::Outdated { version }
    } else {
        HookStatus::Installed { version }
    }
}

/// Appends Hide's entries to every event it registers, preserving everything
/// already there.
///
/// `helper` is the absolute path of the `hide-agent-hooks` executable the
/// hook will run. It is written into the command verbatim, so the caller
/// hands the path of the bundle that is actually running.
pub fn install<'a>(
    runtime: AgentRuntime,
    home: &Path,
    helper: impl Into<HookTarget<'a>>,
) -> Result<InstallOutcome, InstallFailure> {
    let helper = helper.into();
    let path = runtime.config_path(home);
    let mut document =
        read_document(&path)?.unwrap_or_else(|| HookDocument::new(Value::Object(Map::new())));
    let before = document.value.clone();
    let hooks = hooks_object_mut(&mut document, &path)?;
    let mut preserved = 0usize;
    for event in HookEvent::ALL {
        let groups = event_array_mut(hooks, event, &path)?;
        // Remove Hide's own entries first so a second install converges on
        // one entry per event rather than appending another
        // (engineering rule 11, PRD B25).
        strip_owned(groups);
        preserved += groups.len();
        groups.push(hook_group(helper, runtime, event));
    }
    if before == document.value {
        return Ok(InstallOutcome {
            changed: false,
            preserved_entries: preserved,
        });
    }
    write_document(&path, &document)?;
    Ok(InstallOutcome {
        changed: true,
        preserved_entries: preserved,
    })
}

/// Whether each owned entry is exactly the command this target installs.
pub fn matches_install<'a>(
    runtime: AgentRuntime,
    home: &Path,
    helper: impl Into<HookTarget<'a>>,
) -> bool {
    let helper = helper.into();
    let Ok(Some(document)) = read_document(&runtime.config_path(home)) else {
        return false;
    };
    HookEvent::ALL.into_iter().all(|event| {
        let Some(groups) = document
            .get(HOOKS_KEY)
            .and_then(|hooks| hooks.get(event.name()))
            .and_then(Value::as_array)
        else {
            return false;
        };
        let mut owned = groups
            .iter()
            .filter(|group| group_marker_version(group).is_some());
        owned.next() == Some(&hook_group(helper, runtime, event)) && owned.next().is_none()
    })
}

/// A generated entry for this helper at a previous state route is stale;
/// arbitrary edits are not permission for the kit to replace the entry.
pub fn matches_helper(runtime: AgentRuntime, home: &Path, helper: &Path) -> bool {
    let Ok(Some(document)) = read_document(&runtime.config_path(home)) else {
        return false;
    };
    let mut state = None;
    let Some(hooks) = document.get(HOOKS_KEY).and_then(Value::as_object) else {
        return false;
    };
    for group in hooks
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter(|group| group_marker_version(group).is_some())
    {
        for hook in group
            .get("hooks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            for text in hook_texts(hook) {
                match crate::route::in_command(text) {
                    Ok(Some(found)) => state = Some(found),
                    Ok(None) => {}
                    Err(()) => return false,
                }
            }
        }
    }
    match state {
        Some(state) => matches_install(runtime, home, HookTarget::new(helper, &state)),
        // This version always installs a route. Removing it is an edit;
        // older unbound versions are upgraded by the marker-version check.
        None => false,
    }
}

/// Takes out only the entries carrying Hide's marker.
pub fn remove(runtime: AgentRuntime, home: &Path) -> Result<RemoveOutcome, InstallFailure> {
    let path = runtime.config_path(home);
    let Some(mut document) = read_document(&path)? else {
        return Ok(RemoveOutcome {
            changed: false,
            removed_entries: 0,
            preserved_entries: 0,
        });
    };
    let hooks = hooks_object_mut(&mut document, &path)?;
    let mut removed = 0usize;
    let mut preserved = 0usize;
    let mut emptied = Vec::new();
    for (event, value) in hooks.iter_mut() {
        let Some(groups) = value.as_array_mut() else {
            continue;
        };
        let taken = strip_owned(groups);
        removed += taken;
        preserved += groups.len();
        if taken != 0 && groups.is_empty() {
            emptied.push(event.clone());
        }
    }
    for event in emptied {
        // Only an event Hide emptied is dropped; an event another tool left
        // empty was already empty before this ran.
        if HookEvent::parse(&event).is_some() {
            hooks.shift_remove(&event);
        }
    }
    if removed == 0 {
        return Ok(RemoveOutcome {
            changed: false,
            removed_entries: 0,
            preserved_entries: preserved,
        });
    }
    write_document(&path, &document)?;
    Ok(RemoveOutcome {
        changed: true,
        removed_entries: removed,
        preserved_entries: preserved,
    })
}

/// The entry Hide writes: one command hook, carrying its own marker, in the
/// form this system's runtimes run a hook command (`docs/agent-hooks.md`,
/// Installing).
///
/// The command runs the helper only while it is there. A hook outlives the
/// bundle or helper folder that wrote it: the app is moved to the Trash, a
/// device is removed while it is offline, a helper folder is cleaned by hand.
/// A bare path would then fail every one of the agent's turns with `No such
/// file or directory`, which is what happened on 2026-09-10; guarded, a
/// missing helper is a hook that does nothing and succeeds (PRD
/// device-parity D-11, B3).
fn hook_group<'a>(
    helper: impl Into<HookTarget<'a>>,
    runtime: AgentRuntime,
    event: HookEvent,
) -> Value {
    let helper = helper.into();
    let hook = if cfg!(windows) {
        windows_hook(helper, runtime, event)
    } else {
        posix_hook(helper, runtime, event)
    };
    let mut group = serde_json::json!({ "hooks": [hook] });
    if let Some(matcher) = hook_matcher(runtime, event) {
        group["matcher"] = Value::from(matcher);
    }
    group
}

/// The matcher Hide writes on the group of `event`'s entry, `None` for an
/// entry that fires on every occurrence. Both the writer above and the Codex
/// trust step read it here, so an event that has a matcher is written and
/// recognised as Hide's by the same line.
///
/// Select the shell, native questions and the file tools the guardrails read
/// (PRD agent-guardrails D-12): Claude Code 2.1.300's `Edit`, `Write`,
/// `MultiEdit`, `NotebookEdit` and `Read`, and Codex 0.162.1's `apply_patch`,
/// which its `Edit` and `Write` aliases also select. Pinned Codex 0.160.1
/// sends real plan-mode request_user_input through PreToolUse as well, and
/// 0.162.1 asks with `request_user_input_async`. Codex matches a tool by its
/// whole name (a hook on `request_user_input` stayed silent for the async
/// tool, measured 2026-10-15), so each question tool is named.
pub(crate) fn hook_matcher(runtime: AgentRuntime, event: HookEvent) -> Option<&'static str> {
    match (runtime, event) {
        (AgentRuntime::ClaudeCode, HookEvent::PreToolUse) => {
            Some("Bash|AskUserQuestion|ExitPlanMode|Edit|Write|MultiEdit|NotebookEdit|Read")
        }
        (AgentRuntime::Codex, HookEvent::PreToolUse) => {
            Some("Bash|request_user_input|request_user_input_async|apply_patch|Edit|Write")
        }
        _ => None,
    }
}

/// The arguments the helper runs with, after its path. The guard reads no
/// Memory, so its entry does not ask for the Memory input.
fn helper_arguments(helper: HookTarget<'_>, runtime: AgentRuntime, event: HookEvent) -> String {
    let memory = if event == HookEvent::PreToolUse {
        ""
    } else {
        " --memory-injection"
    };
    format!(
        "hook --runtime {} --event {}{memory} --source {}{}",
        runtime.id(),
        event.name(),
        hook_source_id(),
        helper.arguments()
    )
}

/// macOS and Linux: both runtimes hand the command to a POSIX shell (Claude
/// Code `sh -c`, Codex the session's shell, zsh or bash, with `-c`).
fn posix_hook<'a>(
    helper: impl Into<HookTarget<'a>>,
    runtime: AgentRuntime,
    event: HookEvent,
) -> Value {
    let helper = helper.into();
    serde_json::json!({
        "type": "command",
        "command": posix_command(helper, runtime, event),
        "timeout": 8,
    })
}

fn posix_command<'a>(
    helper: impl Into<HookTarget<'a>>,
    runtime: AgentRuntime,
    event: HookEvent,
) -> String {
    let helper = helper.into();
    let quoted = Quoting::Posix.quote(&helper.display().to_string());
    format!(
        "if [ -x {quoted} ]; then exec {quoted} {}; fi",
        helper_arguments(helper, runtime, event)
    )
}

/// Windows: one PowerShell guard for both runtimes. Codex runs a command in
/// the session's shell, which on Windows is PowerShell (`-NoProfile
/// -Command`), so the guard is the command. Claude Code runs a command
/// through Git Bash when it is installed and PowerShell otherwise, so its
/// entry names PowerShell itself in exec form (`args`), which no shell
/// re-parses and which runs the same on every machine.
fn windows_hook<'a>(
    helper: impl Into<HookTarget<'a>>,
    runtime: AgentRuntime,
    event: HookEvent,
) -> Value {
    let helper = helper.into();
    let guard = windows_guard(helper, runtime, event);
    match runtime {
        AgentRuntime::ClaudeCode => serde_json::json!({
            "type": "command",
            "command": "powershell.exe",
            "args": ["-NoProfile", "-NonInteractive", "-Command", guard],
            "timeout": 8,
        }),
        AgentRuntime::Codex => serde_json::json!({
            "type": "command",
            "command": guard,
            "timeout": 8,
        }),
    }
}

fn windows_guard<'a>(
    helper: impl Into<HookTarget<'a>>,
    runtime: AgentRuntime,
    event: HookEvent,
) -> String {
    let helper = helper.into();
    let quoted = Quoting::PowerShell.quote(&helper.display().to_string());
    format!(
        "if (Test-Path -LiteralPath {quoted} -PathType Leaf) {{ & {quoted} {} }}",
        helper_arguments(helper, runtime, event)
    )
}

/// The `command` of the Codex entry [`install`] writes for `event` on this
/// system. Codex takes the command string itself on every system, so this is
/// the whole of what Codex hashes and shows for the entry; the trust step
/// compares against it instead of rebuilding it (`crate::codex_trust`).
pub(crate) fn codex_command<'a>(helper: impl Into<HookTarget<'a>>, event: HookEvent) -> String {
    let helper = helper.into();
    if cfg!(windows) {
        windows_guard(helper, AgentRuntime::Codex, event)
    } else {
        posix_command(helper, AgentRuntime::Codex, event)
    }
}

/// `value` quoted for this system's own shell (PowerShell on Windows, a
/// POSIX shell elsewhere), for a command the operator is shown to type.
pub fn quote_for_shell(value: &str) -> String {
    Quoting::NATIVE.quote(value)
}

/// How a path is quoted in the command this system writes, and read back.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Quoting {
    /// Single quotes, a quote inside written `'\''`.
    Posix,
    /// Single quotes, a quote inside doubled. PowerShell takes the
    /// typographic single quotes for `'` as well, so they are doubled too.
    PowerShell,
    /// Double quotes in agy's legacy Windows commands, retained for reading
    /// and migrating the entries written before guidance version 7.
    Cmd,
}

impl Quoting {
    pub(crate) const NATIVE: Self = if cfg!(windows) {
        Self::PowerShell
    } else {
        Self::Posix
    };

    pub(crate) const POWERSHELL_QUOTES: [char; 5] =
        ['\'', '\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'];

    pub(crate) fn quote(self, value: &str) -> String {
        match self {
            Self::Cmd => format!("\"{value}\""),
            Self::Posix => format!("'{}'", value.replace('\'', "'\\''")),
            Self::PowerShell => {
                let mut quoted = String::with_capacity(value.len() + 2);
                quoted.push('\'');
                for character in value.chars() {
                    if Self::POWERSHELL_QUOTES.contains(&character) {
                        quoted.push(character);
                    }
                    quoted.push(character);
                }
                quoted.push('\'');
                quoted
            }
        }
    }

    /// The first quoted word of `command`, unquoted: the helper path, in the
    /// guarded command and in the bare one versions before 6 wrote.
    pub(crate) fn first_quoted(self, command: &str) -> Option<String> {
        let opening = if self == Self::Cmd { '"' } else { '\'' };
        let start = command.find(opening)? + 1;
        let mut rest = &command[start..];
        let mut path = String::new();
        match self {
            Self::Posix => loop {
                let end = rest.find('\'')?;
                path.push_str(&rest[..end]);
                rest = &rest[end + 1..];
                match rest.strip_prefix("\\''") {
                    Some(after) => {
                        path.push('\'');
                        rest = after;
                    }
                    None => return Some(path),
                }
            },
            Self::PowerShell => {
                let mut characters = rest.chars().peekable();
                while let Some(character) = characters.next() {
                    if !Self::POWERSHELL_QUOTES.contains(&character) {
                        path.push(character);
                    } else if characters.peek() == Some(&character) {
                        path.push(character);
                        characters.next();
                    } else {
                        return Some(path);
                    }
                }
                None
            }
            Self::Cmd => rest.find('"').map(|end| rest[..end].to_owned()),
        }
    }
}

/// The strings of one hook a runtime runs: its `command`, then its exec-form
/// `args`, where Claude Code's Windows entry carries the guard.
fn hook_texts(hook: &Value) -> impl Iterator<Item = &str> {
    let command = hook.get("command").and_then(Value::as_str);
    let args = hook
        .get("args")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    command.into_iter().chain(args)
}

fn group_marker_version(group: &Value) -> Option<u32> {
    group
        .get("hooks")?
        .as_array()?
        .iter()
        .flat_map(hook_texts)
        .filter_map(marker_version_in)
        .min()
}

fn hook_marker_version(hook: &Value) -> Option<u32> {
    hook_texts(hook).filter_map(marker_version_in).min()
}

/// A group can belong to several writers. Only a marked command is Hide's.
fn strip_owned(groups: &mut Vec<Value>) -> usize {
    let mut removed = 0;
    groups.retain_mut(|group| {
        let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            return true;
        };
        let before = hooks.len();
        hooks.retain(|hook| hook_marker_version(hook).is_none());
        let taken = before - hooks.len();
        removed += taken;
        taken == 0 || !hooks.is_empty()
    });
    removed
}

fn owned_versions(groups: &[Value]) -> Vec<u32> {
    groups.iter().filter_map(group_marker_version).collect()
}

/// The helper path recorded in the first entry Hide owns.
fn installed_helper(hooks: &Map<String, Value>) -> Option<String> {
    hooks
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter(|group| group_marker_version(group).is_some())
        .filter_map(|group| group.get("hooks")?.as_array())
        .flatten()
        .filter(|hook| hook_marker_version(hook).is_some())
        .find_map(|hook| {
            hook_texts(hook)
                .filter(|text| text.contains('\''))
                .find_map(|text| Quoting::NATIVE.first_quoted(text))
        })
}

/// The helper Hide's entries in `runtime`'s file name, when there are any.
/// The kit compares it with the helper it would install, so an entry left by
/// an app at another path is replaced rather than taken as current.
pub fn installed_helper_path(runtime: AgentRuntime, home: &Path) -> Option<String> {
    let document = read_document(&runtime.config_path(home)).ok()??;
    let hooks = hooks_object(&document, &runtime.config_path(home)).ok()??;
    installed_helper(hooks)
}

/// The common read/modify/write boundary for instrumented and guidance hooks.
/// Keep the source alongside the semantic value until replacement; rereading
/// only after mutation cannot distinguish an intervening writer's changes.
pub(crate) struct HookDocument {
    value: Value,
    source: Option<String>,
    target: Option<PathBuf>,
}

impl HookDocument {
    pub(crate) fn new(value: Value) -> Self {
        Self {
            value,
            source: None,
            target: None,
        }
    }
}

impl Deref for HookDocument {
    type Target = Value;

    fn deref(&self) -> &Value {
        &self.value
    }
}

impl DerefMut for HookDocument {
    fn deref_mut(&mut self) -> &mut Value {
        &mut self.value
    }
}

impl Serialize for HookDocument {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value.serialize(serializer)
    }
}

// Configuration reads and all source/tree allocations have a finite input
// bound, in addition to serde_json's nesting limit. Oversized files are left
// untouched and reported through the existing installation failure surface.
const MAX_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024;

fn read_source(path: &Path) -> std::io::Result<Option<String>> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut raw = String::new();
    file.take(MAX_DOCUMENT_BYTES + 1).read_to_string(&mut raw)?;
    if raw.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(std::io::Error::other(
            "the hook file exceeds the 16 MiB configuration limit",
        ));
    }
    Ok(Some(raw))
}

pub(crate) fn read_document(path: &Path) -> Result<Option<HookDocument>, InstallFailure> {
    let unreadable = |error: std::io::Error| InstallFailure::Unreadable {
        path: path.display().to_string(),
        detail: error.to_string(),
    };
    let target = match hide_platform::fs::identity::canonical(path) {
        Ok(target) => target,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(unreadable(error)),
    };
    let Some(raw) = read_source(&target).map_err(unreadable)? else {
        return Err(unreadable(std::io::Error::other(
            "the hook file disappeared while Hide was reading it; retry",
        )));
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    crate::lossless_json::parse(&raw)
        .map(|value| {
            Some(HookDocument {
                value,
                source: Some(raw),
                target: Some(target),
            })
        })
        .map_err(|error| InstallFailure::Unparsable {
            path: path.display().to_string(),
            detail: error.to_string(),
        })
}

fn hooks_object<'a>(
    document: &'a Value,
    path: &Path,
) -> Result<Option<&'a Map<String, Value>>, InstallFailure> {
    let Some(object) = document.as_object() else {
        return Err(InstallFailure::UnexpectedShape {
            path: path.display().to_string(),
            detail: "the document is not a JSON object".to_owned(),
        });
    };
    match object.get(HOOKS_KEY) {
        None => Ok(None),
        Some(Value::Object(hooks)) => Ok(Some(hooks)),
        Some(_) => Err(InstallFailure::UnexpectedShape {
            path: path.display().to_string(),
            detail: "\"hooks\" is not an object".to_owned(),
        }),
    }
}

fn hooks_object_mut<'a>(
    document: &'a mut Value,
    path: &Path,
) -> Result<&'a mut Map<String, Value>, InstallFailure> {
    let unexpected = |detail: &str| InstallFailure::UnexpectedShape {
        path: path.display().to_string(),
        detail: detail.to_owned(),
    };
    let object = document
        .as_object_mut()
        .ok_or_else(|| unexpected("the document is not a JSON object"))?;
    let entry = object
        .entry(HOOKS_KEY.to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    entry
        .as_object_mut()
        .ok_or_else(|| unexpected("\"hooks\" is not an object"))
}

fn event_array_mut<'a>(
    hooks: &'a mut Map<String, Value>,
    event: HookEvent,
    path: &Path,
) -> Result<&'a mut Vec<Value>, InstallFailure> {
    let entry = hooks
        .entry(event.name().to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    entry
        .as_array_mut()
        .ok_or_else(|| InstallFailure::UnexpectedShape {
            path: path.display().to_string(),
            detail: format!("\"hooks.{}\" is not an array", event.name()),
        })
}

/// Writes through a temporary file beside the file it replaces, so a failure
/// part way through leaves the operator's original file intact. The file
/// keeps its permission bits, because a settings file kept at 0600 can hold
/// tokens, and a new one is 0600. A settings file that is a link, as a
/// dotfile manager makes it, is written where the link leads, so the link
/// stays the operator's.
pub(crate) fn write_document(path: &Path, document: &HookDocument) -> Result<(), InstallFailure> {
    let failure = |detail: String| InstallFailure::NotWritable {
        path: path.display().to_string(),
        detail,
    };
    let target = match hide_platform::fs::identity::canonical(path) {
        Ok(real) => real,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
                return Err(failure(
                    "the settings symlink has no existing target; repair it and retry".to_owned(),
                ));
            }
            path.to_path_buf()
        }
        Err(error) => return Err(failure(error.to_string())),
    };
    if document
        .target
        .as_ref()
        .is_some_and(|original| original != &target)
    {
        return Err(failure(
            "the settings symlink changed while Hide was editing it; retry".to_owned(),
        ));
    }
    let serialized = if let Some(source) = &document.source {
        crate::lossless_json::rewrite(source, &document.value)
            .map_err(|error| failure(error.to_string()))?
    } else {
        let mut serialized = serde_json::to_string_pretty(&document.value)
            .map_err(|error| failure(error.to_string()))?;
        serialized.push('\n');
        serialized
    };
    if serialized.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(failure(
            "the edited hook file exceeds the 16 MiB configuration limit".to_owned(),
        ));
    }
    // The kit already holds its account lock across this whole operation.
    // This comparison also refuses an observed edit by a noncooperating
    // writer, but is not an OS compare-and-swap against an external writer
    // racing the atomic replacement after this check.
    let current = read_source(&target).map_err(|error| failure(error.to_string()))?;
    let unchanged = match (&document.source, &current) {
        (Some(before), Some(current)) => before == current,
        (None, current) => current
            .as_ref()
            .is_none_or(|source| source.trim().is_empty()),
        _ => false,
    };
    if !unchanged {
        return Err(failure(
            "the hook file changed while Hide was editing it; retry".to_owned(),
        ));
    }
    let parent = target
        .parent()
        .ok_or_else(|| failure("the path has no parent directory".to_owned()))?;
    fs::create_dir_all(parent).map_err(|error| failure(error.to_string()))?;
    hide_platform::fs::atomic::write_file(
        &target,
        serialized.as_bytes(),
        hide_platform::fs::Access::KeepOrPrivate,
    )
    .map(drop)
    .map_err(|error| failure(error.to_string()))
}

#[cfg(test)]
#[path = "install/byte_contract_tests.rs"]
mod byte_contract_tests;

#[cfg(test)]
#[path = "install/source_conflict_tests.rs"]
mod source_conflict_tests;

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn hide_can_take_out_its_own_entries_after_the_helper_they_name_is_gone() {
        let fixture = Fixture::new("helper-gone");
        let runtime = AgentRuntime::ClaudeCode;
        fs::create_dir_all(runtime.home_directory(fixture.home())).unwrap();
        let helper = fixture
            .home()
            .join("hide.app/Contents/Resources/hide-agent-hooks");
        fs::create_dir_all(helper.parent().unwrap()).unwrap();
        fs::write(&helper, b"binary").unwrap();
        install(runtime, fixture.home(), &helper).unwrap();

        // The build directory goes away, which is the whole incident.
        fs::remove_file(&helper).unwrap();
        let diagnosed = crate::Diagnosis::read(fixture.home());
        let row = diagnosed
            .runtimes
            .iter()
            .find(|row| row.runtime == runtime)
            .expect("the runtime is diagnosed");
        assert!(
            matches!(
                row.status,
                HookStatus::Failed {
                    reason: InstallFailure::HelperMissing { .. }
                }
            ),
            "got {:?}",
            row.status
        );
        assert!(
            row.offers_removal(),
            "the operator can take out entries whose helper is gone"
        );

        // And removal works without the binary, because it reads the file.
        let outcome = remove(runtime, fixture.home()).unwrap();
        assert!(outcome.changed);
        assert_eq!(outcome.removed_entries, HookEvent::ALL.len());
        assert!(matches!(
            status(runtime, fixture.home()),
            HookStatus::NotInstalled
        ));
    }

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "hide-agent-hooks-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|value| value.as_nanos())
                    .unwrap_or_default()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join(".claude")).unwrap();
            fs::create_dir_all(root.join(".codex")).unwrap();
            Self(root)
        }

        fn home(&self) -> &Path {
            &self.0
        }

        fn write(&self, runtime: AgentRuntime, body: &str) {
            fs::write(runtime.config_path(self.home()), body).unwrap();
        }

        fn read(&self, runtime: AgentRuntime) -> Value {
            let raw = fs::read_to_string(runtime.config_path(self.home())).unwrap();
            serde_json::from_str(&raw).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn helper(fixture: &Fixture) -> PathBuf {
        let path = fixture.home().join(crate::HELPER_BINARY_NAME);
        fs::write(&path, "#!/bin/sh\n").unwrap();
        path
    }

    /// The occupied array is the whole reason this crate exists: two other
    /// tools already sit on `SubagentStart` (PRD D-26).
    const OCCUPIED: &str = r#"{
      "model": "opus",
      "hooks": {
        "SubagentStart": [
          {"hooks": [{"type": "command", "command": "/opt/principles/inject.sh SubagentStart", "timeout": 5}]},
          {"hooks": [{"type": "command", "command": "/opt/orca/codex-hook.sh", "timeout": 10}]}
        ],
        "Stop": [
          {"hooks": [{"type": "command", "command": "/opt/checkpoint.sh"}]}
        ]
      }
    }"#;

    #[test]
    fn installing_appends_and_leaves_every_other_tools_entry_exactly_as_it_was() {
        let fixture = Fixture::new("append");
        fixture.write(AgentRuntime::Codex, OCCUPIED);
        let before = fixture.read(AgentRuntime::Codex);

        let outcome = install(AgentRuntime::Codex, fixture.home(), &helper(&fixture)).unwrap();
        assert!(outcome.changed);
        assert_eq!(outcome.preserved_entries, 3);

        let after = fixture.read(AgentRuntime::Codex);
        for event in ["SubagentStart", "Stop"] {
            let original = before["hooks"][event].as_array().unwrap();
            let current = after["hooks"][event].as_array().unwrap();
            assert_eq!(
                current.len(),
                original.len() + 1,
                "{event} gained one entry"
            );
            assert_eq!(
                &current[..original.len()],
                original.as_slice(),
                "{event} kept its own"
            );
        }
        assert_eq!(after["model"], before["model"], "unrelated keys survive");
        assert!(after["hooks"]["SessionStart"].as_array().unwrap().len() == 1);
        assert!(after["hooks"]["SubagentStop"].as_array().unwrap().len() == 1);
        let session_start_command = after["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(session_start_command.contains("--runtime codex"));
        assert!(session_start_command.contains("--memory-injection"));
        assert!(session_start_command.contains("--source hide-subagents@9"));
        let guard = if cfg!(windows) {
            "if (Test-Path -LiteralPath '"
        } else {
            "if [ -x '"
        };
        assert!(
            session_start_command.starts_with(guard),
            "the command runs the helper only while it is there"
        );
    }

    /// The unbound command bytes on macOS and Linux. Changing them makes
    /// every installed entry read as another build's, so they are pinned as
    /// independent literals, quote escaping included.
    #[test]
    fn the_posix_entry_is_exactly_what_macos_and_linux_have_installed() {
        let helper =
            Path::new("/Users/example/o'brien/hide.app/Contents/Resources/hide-agent-hooks");
        let pinned = [
            (
                AgentRuntime::ClaudeCode,
                HookEvent::SessionStart,
                r#"{"type":"command","command":"if [ -x '/Users/example/o'\\''brien/hide.app/Contents/Resources/hide-agent-hooks' ]; then exec '/Users/example/o'\\''brien/hide.app/Contents/Resources/hide-agent-hooks' hook --runtime claude-code --event SessionStart --memory-injection --source hide-subagents@9; fi","timeout":8}"#,
            ),
            (
                AgentRuntime::Codex,
                HookEvent::Stop,
                r#"{"type":"command","command":"if [ -x '/Users/example/o'\\''brien/hide.app/Contents/Resources/hide-agent-hooks' ]; then exec '/Users/example/o'\\''brien/hide.app/Contents/Resources/hide-agent-hooks' hook --runtime codex --event Stop --memory-injection --source hide-subagents@9; fi","timeout":8}"#,
            ),
        ];
        for (runtime, event, bytes) in pinned {
            let written = posix_hook(helper, runtime, event);
            assert_eq!(serde_json::to_string(&written).unwrap(), bytes);
            let command = written["command"].as_str().unwrap();
            assert_eq!(
                Quoting::Posix.first_quoted(command).as_deref(),
                helper.to_str()
            );
        }
        if !cfg!(windows) {
            assert_eq!(
                hook_group(helper, AgentRuntime::Codex, HookEvent::Stop),
                serde_json::json!({ "hooks": [posix_hook(helper, AgentRuntime::Codex, HookEvent::Stop)] })
            );
        }
    }

    /// Windows: Claude Code's entry runs PowerShell in exec form, Codex's
    /// command is the PowerShell guard its session shell runs. A path with
    /// a space, a quote and a typographic quote PowerShell also takes for
    /// one is read back as it was written, and the marker is found in
    /// either shape.
    #[test]
    fn the_windows_entries_are_powershell_guards_hide_reads_back() {
        let helper = Path::new("C:\\Users\\example\\a b'c\\it\u{2019}s\\hide-agent-hooks.exe");
        let quoted = "'C:\\Users\\example\\a b''c\\it\u{2019}\u{2019}s\\hide-agent-hooks.exe'";
        let guard = |runtime: &str| {
            format!(
                "if (Test-Path -LiteralPath {quoted} -PathType Leaf) {{ & {quoted} hook --runtime {runtime} --event Stop --memory-injection --source hide-subagents@9 }}"
            )
        };
        let codex = windows_hook(helper, AgentRuntime::Codex, HookEvent::Stop);
        assert_eq!(
            codex,
            serde_json::json!({ "type": "command", "command": guard("codex"), "timeout": 8 })
        );
        let claude = windows_hook(helper, AgentRuntime::ClaudeCode, HookEvent::Stop);
        assert_eq!(
            claude,
            serde_json::json!({
                "type": "command",
                "command": "powershell.exe",
                "args": ["-NoProfile", "-NonInteractive", "-Command", guard("claude-code")],
                "timeout": 8,
            })
        );
        for hook in [claude, codex] {
            let group = serde_json::json!({ "hooks": [hook] });
            assert_eq!(
                group_marker_version(&group),
                Some(crate::runtime::HOOK_ENTRY_VERSION)
            );
            let text = hook_texts(&group["hooks"][0])
                .find(|text| text.contains('\''))
                .unwrap();
            assert_eq!(
                Quoting::PowerShell.first_quoted(text).as_deref(),
                helper.to_str()
            );
        }
        if cfg!(windows) {
            assert_eq!(
                hook_group(helper, AgentRuntime::Codex, HookEvent::Stop),
                serde_json::json!({ "hooks": [windows_hook(helper, AgentRuntime::Codex, HookEvent::Stop)] })
            );
        }
    }

    #[test]
    fn installing_the_same_thing_twice_leaves_one_entry_per_event() {
        let fixture = Fixture::new("idempotent");
        fixture.write(AgentRuntime::Codex, OCCUPIED);
        let path = helper(&fixture);
        install(AgentRuntime::Codex, fixture.home(), &path).unwrap();
        let first = fixture.read(AgentRuntime::Codex);
        let second_outcome = install(AgentRuntime::Codex, fixture.home(), &path).unwrap();
        assert!(
            !second_outcome.changed,
            "a converged install writes nothing"
        );
        assert_eq!(fixture.read(AgentRuntime::Codex), first);
        for event in HookEvent::ALL {
            let groups = first["hooks"][event.name()].as_array().unwrap();
            assert_eq!(
                owned_versions(groups).len(),
                1,
                "{} has one Hide entry",
                event.name()
            );
        }
    }

    #[test]
    fn the_codex_command_the_trust_step_expects_is_what_install_writes() {
        // Codex hashes the command string it finds in the file, so the trust
        // step recognises Hide's entry by `codex_command` alone: on every
        // system it must be exactly what `install` wrote for each event, with
        // the matcher `hook_matcher` names.
        let fixture = Fixture::new("codex-command");
        let path = helper(&fixture);
        install(AgentRuntime::Codex, fixture.home(), &path).unwrap();
        let written = fixture.read(AgentRuntime::Codex);
        for event in HookEvent::ALL {
            let group = &written["hooks"][event.name()][0];
            assert_eq!(
                group["hooks"][0]["command"],
                codex_command(&path, event),
                "{}",
                event.name()
            );
            assert_eq!(
                group.get("matcher").and_then(Value::as_str),
                hook_matcher(AgentRuntime::Codex, event),
                "{}",
                event.name()
            );
        }
    }

    /// Another tool's `PreToolUse` entries, which the guard's event shares.
    const OTHER_PRE_TOOL_USE: &str = r#"{
      "hooks": {
        "PreToolUse": [
          {"matcher": "Bash", "hooks": [{"type": "command", "command": "/opt/guard/root-only.sh", "timeout": 10}]},
          {"hooks": [{"type": "command", "command": "/opt/audit/every-tool.sh"}]}
        ]
      }
    }"#;

    #[test]
    fn the_guard_entry_selects_shell_and_native_questions_and_asks_for_no_memory() {
        for runtime in AgentRuntime::ALL {
            let fixture = Fixture::new("guard-entry");
            install(runtime, fixture.home(), &helper(&fixture)).unwrap();
            let written = fixture.read(runtime);
            let group = &written["hooks"]["PreToolUse"][0];
            let expected = match runtime {
                AgentRuntime::ClaudeCode => {
                    "Bash|AskUserQuestion|ExitPlanMode|Edit|Write|MultiEdit|NotebookEdit|Read"
                }
                AgentRuntime::Codex => {
                    "Bash|request_user_input|request_user_input_async|apply_patch|Edit|Write"
                }
            };
            assert_eq!(group["matcher"], expected, "{runtime:?}");
            let hook = &group["hooks"][0];
            let text = hook_texts(hook).collect::<Vec<_>>().join(" ");
            assert!(text.contains("--event PreToolUse"), "{text}");
            assert!(!text.contains("--memory-injection"), "{text}");
            assert!(text.contains(&hook_source_id()), "{text}");
            // The other five events keep the form version 6 wrote.
            let start = hook_texts(&written["hooks"]["SessionStart"][0]["hooks"][0])
                .collect::<Vec<_>>()
                .join(" ");
            assert!(start.contains("--memory-injection"), "{start}");
            assert!(written["hooks"]["SessionStart"][0].get("matcher").is_none());
        }
    }

    #[test]
    fn installing_beside_another_tools_pre_tool_use_entries_keeps_them_and_converges() {
        for runtime in AgentRuntime::ALL {
            let fixture = Fixture::new("guard-shared");
            fixture.write(runtime, OTHER_PRE_TOOL_USE);
            let path = helper(&fixture);
            let first = install(runtime, fixture.home(), &path).unwrap();
            assert!(first.changed);
            let after = fixture.read(runtime);
            let groups = after["hooks"]["PreToolUse"].as_array().unwrap();
            assert_eq!(groups.len(), 3, "two of theirs and one of Hide's");
            assert_eq!(groups[0]["hooks"][0]["command"], "/opt/guard/root-only.sh");
            assert_eq!(groups[1]["hooks"][0]["command"], "/opt/audit/every-tool.sh");
            assert!(group_marker_version(&groups[2]).is_some());
            let second = install(runtime, fixture.home(), &path).unwrap();
            assert!(!second.changed, "a second install writes nothing");
            assert_eq!(fixture.read(runtime), after);
            let removed = remove(runtime, fixture.home()).unwrap();
            assert_eq!(removed.removed_entries, HookEvent::ALL.len());
            let left = fixture.read(runtime);
            assert_eq!(
                left["hooks"]["PreToolUse"].as_array().unwrap().len(),
                2,
                "only Hide's entry left with the guard, theirs stay"
            );
        }
    }

    #[test]
    fn a_file_with_the_five_earlier_events_reads_outdated_until_the_guard_is_added() {
        // What every machine that installed Hide before the guard has.
        for runtime in AgentRuntime::ALL {
            let fixture = Fixture::new("guard-upgrade");
            let path = helper(&fixture);
            install(runtime, fixture.home(), &path).unwrap();
            let mut document = fixture.read(runtime);
            document["hooks"]
                .as_object_mut()
                .unwrap()
                .remove("PreToolUse");
            fixture.write(runtime, &serde_json::to_string_pretty(&document).unwrap());
            assert!(
                matches!(status(runtime, fixture.home()), HookStatus::Outdated { version } if version == crate::runtime::HOOK_ENTRY_VERSION),
                "{runtime:?}"
            );
            install(runtime, fixture.home(), &path).unwrap();
            assert!(matches!(
                status(runtime, fixture.home()),
                HookStatus::Installed { .. }
            ));
        }
    }

    #[test]
    fn an_entry_written_with_an_earlier_matcher_reads_outdated_until_it_is_rewritten() {
        // What every Codex machine that installed Hide before
        // `request_user_input_async` has: Codex matches a tool by its whole
        // name (measured 2026-10-15, codex-cli 0.162.1), so `request_user_input`
        // never selects the async tool.
        for runtime in AgentRuntime::ALL {
            let fixture = Fixture::new("matcher-upgrade");
            let path = helper(&fixture);
            install(runtime, fixture.home(), &path).unwrap();
            let mut document = fixture.read(runtime);
            let current = hook_matcher(runtime, HookEvent::PreToolUse).unwrap();
            let earlier = match runtime {
                AgentRuntime::ClaudeCode => current.to_owned(),
                AgentRuntime::Codex => "Bash|request_user_input".to_owned(),
            };
            document["hooks"]["PreToolUse"][0]["matcher"] = Value::from(earlier.clone());
            fixture.write(runtime, &serde_json::to_string_pretty(&document).unwrap());
            let stale = earlier != current;
            assert_eq!(
                matches!(
                    status(runtime, fixture.home()),
                    HookStatus::Outdated { version } if version == crate::runtime::HOOK_ENTRY_VERSION
                ),
                stale,
                "{runtime:?}"
            );
            install(runtime, fixture.home(), &path).unwrap();
            assert!(
                matches!(
                    status(runtime, fixture.home()),
                    HookStatus::Installed { .. }
                ),
                "{runtime:?}"
            );
            assert_eq!(
                fixture.read(runtime)["hooks"]["PreToolUse"][0]["matcher"],
                current
            );
        }
    }

    #[test]
    fn a_file_that_does_not_parse_is_never_written() {
        let fixture = Fixture::new("unparsable");
        let broken = "{\"hooks\": {\"Stop\": [ } ";
        fixture.write(AgentRuntime::Codex, broken);
        let error = install(AgentRuntime::Codex, fixture.home(), &helper(&fixture)).unwrap_err();
        assert!(matches!(error, InstallFailure::Unparsable { .. }));
        assert_eq!(
            fs::read_to_string(AgentRuntime::Codex.config_path(fixture.home())).unwrap(),
            broken,
            "the operator's bytes are untouched"
        );
        assert!(matches!(
            status(AgentRuntime::Codex, fixture.home()),
            HookStatus::Failed {
                reason: InstallFailure::Unparsable { .. }
            }
        ));
    }

    #[test]
    fn removal_takes_only_hides_entries_and_leaves_the_events_it_did_not_empty() {
        let fixture = Fixture::new("remove");
        fixture.write(AgentRuntime::Codex, OCCUPIED);
        install(AgentRuntime::Codex, fixture.home(), &helper(&fixture)).unwrap();

        let outcome = remove(AgentRuntime::Codex, fixture.home()).unwrap();
        assert!(outcome.changed);
        assert_eq!(outcome.removed_entries, HookEvent::ALL.len());
        assert_eq!(outcome.preserved_entries, 3);

        let after = fixture.read(AgentRuntime::Codex);
        let subagent_start = after["hooks"]["SubagentStart"].as_array().unwrap();
        assert_eq!(subagent_start.len(), 2);
        assert!(
            subagent_start[0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains("inject.sh")
        );
        assert_eq!(after["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert!(
            after["hooks"].get("SessionStart").is_none(),
            "an event Hide emptied is dropped"
        );
        assert!(matches!(
            status(AgentRuntime::Codex, fixture.home()),
            HookStatus::NotInstalled
        ));
    }

    #[test]
    fn a_missing_runtime_directory_is_reported_rather_than_created() {
        let fixture = Fixture::new("absent");
        fs::remove_dir_all(fixture.home().join(".codex")).unwrap();
        assert_eq!(
            status(AgentRuntime::Codex, fixture.home()),
            HookStatus::RuntimeAbsent
        );
        assert!(!AgentRuntime::Codex.config_path(fixture.home()).exists());
    }

    #[test]
    fn an_older_marker_version_reads_as_outdated() {
        let fixture = Fixture::new("outdated");
        let helper_path = helper(&fixture);
        let quoted = Quoting::NATIVE.quote(&helper_path.display().to_string());
        let mut document = serde_json::json!({ "hooks": {} });
        for event in HookEvent::ALL {
            document["hooks"][event.name()] = serde_json::json!([{
                "hooks": [{
                    "type": "command",
                    "command": format!("{quoted} hook --event {} --source hide-subagents@0", event.name()),
                }]
            }]);
        }
        fixture.write(AgentRuntime::Codex, &document.to_string());
        assert_eq!(
            status(AgentRuntime::Codex, fixture.home()),
            HookStatus::Outdated { version: 0 }
        );
    }

    #[test]
    fn an_entry_whose_helper_is_gone_is_a_failure_not_a_healthy_install() {
        let fixture = Fixture::new("helper-gone");
        let helper_path = helper(&fixture);
        install(AgentRuntime::Codex, fixture.home(), &helper_path).unwrap();
        fs::remove_file(&helper_path).unwrap();
        assert!(matches!(
            status(AgentRuntime::Codex, fixture.home()),
            HookStatus::Failed {
                reason: InstallFailure::HelperMissing { .. }
            }
        ));
    }

    /// The command as `/bin/sh -c` runs it, which is how both runtimes run a
    /// command hook on Unix.
    #[cfg(unix)]
    fn run_hook_command(command: &str) -> std::process::ExitStatus {
        std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .stdin(std::process::Stdio::null())
            .status()
            .unwrap()
    }

    /// The guard is shell syntax and the helper here is a shell script, so
    /// this runs where `/bin/sh` does.
    #[cfg(unix)]
    #[test]
    fn a_hook_whose_helper_is_gone_succeeds_and_one_that_is_there_runs() {
        let fixture = Fixture::new("guard");
        // A quote in the folder name is the case the path reader must undo.
        let helper = fixture
            .home()
            .join("it's here")
            .join(crate::HELPER_BINARY_NAME);
        install(AgentRuntime::Codex, fixture.home(), &helper).unwrap();
        assert_eq!(
            installed_helper_path(AgentRuntime::Codex, fixture.home()).as_deref(),
            Some(helper.to_str().unwrap()),
            "the path is read back from the guarded command"
        );
        let command = fixture.read(AgentRuntime::Codex)["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .to_owned();

        // The app was moved to the Trash: the agent's turn goes on (B3).
        assert!(run_hook_command(&command).success());

        let ran = fixture.home().join("ran");
        fs::create_dir_all(helper.parent().unwrap()).unwrap();
        fs::write(
            &helper,
            format!("#!/bin/sh\nprintf '%s' \"$*\" > '{}'\n", ran.display()),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(run_hook_command(&command).success());
        let arguments = fs::read_to_string(&ran).unwrap();
        assert!(
            arguments.starts_with("hook --runtime codex --event Stop"),
            "{arguments}"
        );
    }

    /// A settings file can hold tokens, and a dotfile manager can own it as
    /// a link: installing and removing keep its permissions and its link.
    #[test]
    fn installing_and_removing_keep_the_files_permissions_and_its_link() {
        use hide_platform::fs::link;
        use hide_platform::fs::permissions::Permissions;
        let kept = |path: &Path| Permissions::of(&fs::File::open(path).unwrap()).unwrap();
        let fixture = Fixture::new("mode-link");
        let claude = AgentRuntime::ClaudeCode.config_path(fixture.home());
        fixture.write(AgentRuntime::ClaudeCode, OCCUPIED);
        let dotfiles = fixture.home().join("dotfiles");
        fs::create_dir_all(&dotfiles).unwrap();
        let real = dotfiles.join("hooks.json");
        fs::write(&real, OCCUPIED).unwrap();
        // The permissions that matter are the mode bits, and only Unix has
        // them; elsewhere the files keep what the system gave them.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&claude, fs::Permissions::from_mode(0o600)).unwrap();
            fs::set_permissions(&real, fs::Permissions::from_mode(0o640)).unwrap();
        }
        let codex = AgentRuntime::Codex.config_path(fixture.home());
        match link::create_link(&real, &codex) {
            Ok(()) => {}
            // A Windows account with neither the privilege nor Developer
            // Mode cannot make the link this test needs.
            Err(error) if link::needs_privilege(&error) => return,
            Err(error) => panic!("{error}"),
        }
        let (claude_before, real_before) = (kept(&claude), kept(&real));
        #[cfg(unix)]
        {
            assert_eq!(claude_before.unix_mode(), Some(0o600));
            assert_eq!(real_before.unix_mode(), Some(0o640));
        }

        for runtime in [AgentRuntime::ClaudeCode, AgentRuntime::Codex] {
            assert!(
                install(runtime, fixture.home(), &helper(&fixture))
                    .unwrap()
                    .changed
            );
        }
        assert_eq!(kept(&claude), claude_before);
        assert!(link::is_link_to(&codex, &real));
        assert_eq!(kept(&real), real_before);
        assert!(
            fs::read_to_string(&real)
                .unwrap()
                .contains(crate::HELPER_BINARY_NAME)
        );

        for runtime in [AgentRuntime::ClaudeCode, AgentRuntime::Codex] {
            remove(runtime, fixture.home()).unwrap();
        }
        assert_eq!(kept(&claude), claude_before);
        assert!(link::is_link_to(&codex, &real));
        assert_eq!(
            fixture.read(AgentRuntime::Codex),
            serde_json::from_str::<Value>(OCCUPIED).unwrap()
        );
    }

    #[test]
    fn a_runtime_with_no_config_file_yet_installs_into_a_new_one() {
        let fixture = Fixture::new("fresh");
        assert_eq!(
            status(AgentRuntime::ClaudeCode, fixture.home()),
            HookStatus::NotInstalled
        );
        install(AgentRuntime::ClaudeCode, fixture.home(), &helper(&fixture)).unwrap();
        assert_eq!(
            status(AgentRuntime::ClaudeCode, fixture.home()),
            HookStatus::Installed {
                version: crate::runtime::HOOK_ENTRY_VERSION
            }
        );
    }
}
