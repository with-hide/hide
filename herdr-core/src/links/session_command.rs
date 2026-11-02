//! `hide session list|record|read`: an agent's read of the Project's
//! sessions, narrowed in three steps (PRD agent-session-read D-01): one line
//! per session, a session's record as its screen shows it, and a few raw
//! events around one request.
//!
//! The scope is the caller's Project and the connected devices' Projects of
//! the same repository, as each checkout's own `origin` names it, with every
//! descendant of their sessions (D-09). `list` and `record` read the link record on a read-only
//! connection off the runtime's lock; `record` is the screen's own
//! [`LinkStore::ledger_record`], with the pull requests laid by the runtime
//! as the screen's are. `read` asks the node that holds the session's file,
//! through the raw window Hide AI's investigation reads with
//! (`crate::session_excerpt`). Nothing here writes, and the screen's one
//! record slot (`snapshot.session_record`) is never touched (B16).

use super::ledger::{FlowResult, RecordedRequest, SessionIdentity, SessionRecord};
use super::query::ScopeProject;
use super::store::{LinkStore, ListedSession, main_flow, named_by_goal};
use crate::labels::analysis::LabelEnd;
use crate::node_access::NodeLink;
use crate::session_excerpt::{self, Extent, Located, ReadFailure, SessionWindow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The period `list` reads when none is given (D-11).
pub const LIST_SINCE_DEFAULT_MS: u64 = 7 * 24 * 60 * 60 * 1000;
/// The longest period `list` takes.
pub const LIST_SINCE_MAX_MS: u64 = 3_650 * 24 * 60 * 60 * 1000;
pub const LIST_LIMIT_DEFAULT: u32 = 50;
pub const LIST_LIMIT_MAX: u32 = 200;
/// Events `read` shows when none is asked, and the most it shows (D-13).
pub const READ_EVENTS_DEFAULT: u32 = 16;
pub const READ_EVENTS_MAX: u32 = 40;
/// The most characters one line of an event shows.
pub const READ_LINE_CHARS: usize = 400;
/// The characters of a request's own words shown when it has no summary
/// (D-05, as the record screen shows it).
pub const RAW_REQUEST_CHARS: usize = 120;
/// How long `read` waits for the node before it is refused (D-14).
pub const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// The children a record answers, as the record tree holds them.
const CHILDREN_LIMIT: usize = 200;

/// A refusal: the reason code, with the detail its next action names when
/// it carries one ([`DETAILED`]), and the next action the CLI prints.
pub type Refusal = (String, &'static str);

/// The refusals whose code carries a detail for the daemon to put in the
/// next action (`coordination::split_refusal`): the valid range (B11).
pub const DETAILED: &[&str] = &["out_of_range"];

fn no(code: &str, next_action: &'static str) -> Refusal {
    (code.to_owned(), next_action)
}

/// What `hide session` asks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionCommand {
    List {
        #[serde(default)]
        since_ms: Option<u64>,
        #[serde(default)]
        limit: Option<u32>,
    },
    Record {
        id: String,
    },
    Read {
        id: String,
        #[serde(default)]
        request: Option<u32>,
        #[serde(default)]
        from: Option<u32>,
        #[serde(default)]
        events: Option<u32>,
    },
}

impl SessionCommand {
    /// Whether the command is one an agent could have meant; a limit or a
    /// count past its cap is refused here, not shortened (B2, B11).
    pub fn valid(&self) -> bool {
        match self {
            Self::List { since_ms, limit } => {
                since_ms.is_none_or(|since| since > 0 && since <= LIST_SINCE_MAX_MS)
                    && limit.is_none_or(|limit| (1..=LIST_LIMIT_MAX).contains(&limit))
            }
            Self::Record { id } => valid_id(id),
            Self::Read {
                id,
                request,
                from,
                events,
            } => {
                valid_id(id)
                    && (request.is_some() != from.is_some())
                    && events.is_none_or(|n| (1..=READ_EVENTS_MAX).contains(&n))
            }
        }
    }

    fn verb(&self) -> &'static str {
        match self {
            Self::List { .. } => "list",
            Self::Record { .. } => "record",
            Self::Read { .. } => "read",
        }
    }

    fn id(&self) -> Option<&str> {
        match self {
            Self::List { .. } => None,
            Self::Record { id } | Self::Read { id, .. } => Some(id),
        }
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
}

/// A device as the runtime knows it when the command came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceFacts {
    pub id: String,
    /// The name the operator registered it under; none for this machine.
    pub label: Option<String>,
    pub local: bool,
    /// Since when it has not answered; none while it is connected.
    pub unreachable_since_unix_ms: Option<u64>,
}

/// What the runtime hands a `hide session` command, copied under its lock.
#[derive(Clone)]
pub struct SessionScope {
    pub store: PathBuf,
    pub local_device: String,
    /// The record key of the caller checkout's Project; empty when the
    /// caller is in none.
    pub caller: String,
    pub projects: Vec<ScopeProject>,
    /// Each Project's repository by record key: the one the operator
    /// connected it to, else the one its own checkout's `origin` names
    /// (`[HOST/]OWNER/NAME`), read from Git with no GitHub call, so a
    /// device's clone joins the caller's Project even when GitHub was never
    /// read for it (D-09). A Project with neither has no entry.
    pub repositories: BTreeMap<String, String>,
    pub devices: Vec<DeviceFacts>,
    /// This machine's node and each ready device's.
    pub nodes: Vec<(String, Arc<dyn NodeLink>)>,
    /// Hide AI's request summaries are on (`enabled && chosen && agent_summary`).
    pub summaries: bool,
    pub now_unix_ms: u64,
}

/// A session's state now, as its live pane shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveState {
    /// `working` or `needs_you`; none while the pane neither works nor asks.
    pub kind: Option<&'static str>,
    /// The turn's request as the pane names it.
    pub request: Option<String>,
    /// What it does or asks now, in the agent's own words.
    pub now: Option<String>,
}

/// What the runtime holds of one session beyond the record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionFacts {
    /// Its Hide agent id, when it registered.
    pub agent_id: Option<String>,
    pub live: Option<LiveState>,
    pub archived_at_unix_ms: Option<u64>,
}

/// The runtime's part of a command, each asked once under its lock with
/// owned values in and out.
pub trait SessionHost {
    /// The native session the registered agent `id` (`agent-N`) runs.
    fn agent_session(&self, id: &str) -> Option<SessionIdentity>;
    /// The facts of each session, in the order asked; none when the runtime
    /// could not answer.
    fn facts(&self, sessions: Vec<SessionIdentity>) -> Option<Vec<SessionFacts>>;
    /// The record with its `PR` sections laid as the screen lays them; none
    /// when the runtime could not answer.
    fn lay_pulls(&self, record: SessionRecord) -> Option<SessionRecord>;
}

/// Answers `command`; a refusal is logged with the session and device it
/// named, never with conversation words (B21).
pub fn run(
    scope: &SessionScope,
    command: &SessionCommand,
    host: &dyn SessionHost,
) -> Result<Value, Refusal> {
    let mut device = None;
    let result = answer(scope, command, host, &mut device);
    if let Err((reason, _)) = &result {
        let code = crate::coordination::split_refusal(reason).0;
        crate::diagnostic!(json!({
            "component": "session_cli",
            "kind": "session.refused",
            "verb": command.verb(),
            // An id the command refused is logged by its length only.
            "session_id": command.id().filter(|id| valid_id(id)),
            "session_id_bytes": command.id().map(str::len),
            "device_id": device,
            "code": code,
        }));
    }
    result
}

fn answer(
    scope: &SessionScope,
    command: &SessionCommand,
    host: &dyn SessionHost,
    device: &mut Option<String>,
) -> Result<Value, Refusal> {
    if !command.valid() {
        return Err(no(
            "invalid_request",
            "Check hide session arguments: --limit 1-200, --events 1-40, one of --request or --from",
        ));
    }
    let caller = scope
        .projects
        .iter()
        .find(|project| project.key == scope.caller)
        .ok_or_else(|| {
            no(
                "workspace_unavailable",
                "Run hide session from a checkout of a registered Project",
            )
        })?;
    let store = LinkStore::open_read_only(&scope.store).map_err(|code| {
        crate::diagnostic!(
            json!({"component": "session_cli", "kind": "store.read_failed", "code": code})
        );
        if code == "links_store_missing" {
            no(
                "links_unavailable",
                "Hide has not recorded sessions yet; retry after it has read the session files",
            )
        } else {
            no(
                "links_unavailable",
                "Retry; the reason is in Hide's diagnostic log",
            )
        }
    })?;
    let keys = scope_keys(scope, caller);
    let members = store.scope_members(&keys).map_err(failed)?;
    match command {
        SessionCommand::List { since_ms, limit } => {
            list(&store, scope, caller, &members, *since_ms, *limit, host)
        }
        SessionCommand::Record { id } => {
            let session = resolve(&store, &members, host, id, scope)?;
            *device = Some(session.device_id.clone());
            record(&store, scope, &members, &session, host)
        }
        SessionCommand::Read {
            id,
            request,
            from,
            events,
        } => {
            let session = resolve(&store, &members, host, id, scope)?;
            *device = Some(session.device_id.clone());
            read(&store, scope, &session, *request, *from, *events)
        }
    }
}

fn core_gone() -> Refusal {
    no(
        "links_unavailable",
        "Hide is stopping; retry once it runs again",
    )
}

fn failed(code: String) -> Refusal {
    crate::diagnostic!(
        json!({"component": "session_cli", "kind": "store.read_failed", "code": code})
    );
    // A record past one of its caps gives the same answer on every retry.
    match code.as_str() {
        "session_scope_capacity" => {
            return no(
                "links_unavailable",
                "The Project holds more sessions than Hide reads at once, so a retry gives the same answer",
            );
        }
        "ledger_record_request_capacity" | "ledger_descendant_capacity" => {
            return no(
                "links_unavailable",
                "The record is past what Hide reads at once, so a retry gives the same answer; read the session's events with hide session read --from",
            );
        }
        _ => {}
    }
    no(
        "links_unavailable",
        "Retry; the reason is in Hide's diagnostic log",
    )
}

/// The caller's Project and every device's Project of the caller's
/// repository; the caller's alone when its repository is unknown.
fn scope_keys(scope: &SessionScope, caller: &ScopeProject) -> Vec<String> {
    let mut keys = vec![caller.key.clone()];
    let Some(name) = scope.repositories.get(&caller.key) else {
        return keys;
    };
    keys.extend(
        scope
            .projects
            .iter()
            .filter(|project| project.device_id != caller.device_id && project.key != caller.key)
            .filter(|project| {
                scope
                    .repositories
                    .get(&project.key)
                    .is_some_and(|other| other.eq_ignore_ascii_case(name))
            })
            .map(|project| project.key.clone()),
    );
    keys
}

/// The session `id` names: a registered agent's id or a native session id,
/// refused when the record does not hold it or holds it only outside the
/// scope.
fn resolve(
    store: &LinkStore,
    members: &BTreeSet<SessionIdentity>,
    host: &dyn SessionHost,
    id: &str,
    scope: &SessionScope,
) -> Result<SessionIdentity, Refusal> {
    let not_found = || {
        no(
            "not_found",
            "Check the id with hide session list; the record holds only the sessions it has read",
        )
    };
    let known = if is_agent_id(id) {
        let session = host.agent_session(id).ok_or_else(not_found)?;
        vec![session]
    } else {
        store.sessions_named(id).map_err(failed)?
    };
    let mut held = Vec::new();
    for session in known {
        if store.session_file(&session).map_err(failed)?.is_some() {
            held.push(session);
        }
    }
    if held.is_empty() {
        return Err(not_found());
    }
    let mut inside: Vec<_> = held.into_iter().filter(|s| members.contains(s)).collect();
    if inside.is_empty() {
        return Err(no(
            "other_project",
            "The session is in another Project; run hide session from a checkout of that Project",
        ));
    }
    inside.sort_by_key(|session| session.device_id != scope.local_device);
    Ok(inside.swap_remove(0))
}

fn is_agent_id(id: &str) -> bool {
    id.strip_prefix("agent-")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Where a line's words came from: Hide AI's summary, or what was sent or
/// recorded as it was (D-05).
const AI: &str = "ai_summary";
const RAW: &str = "raw";

fn device_value(scope: &SessionScope, id: &str) -> Value {
    let facts = scope.devices.iter().find(|device| device.id == id);
    let local = id == scope.local_device;
    json!({
        "id": id,
        "label": facts.and_then(|device| device.label.clone()),
        "local": local,
        // The last value read is shown with how old it is (D-03).
        "disconnected_since_unix_ms": if local { None } else {
            facts.map_or(Some(0), |device| device.unreachable_since_unix_ms)
        },
    })
}

/// Free text from a session (a title, a request, an agent's own words, a
/// summary) as another agent may read it: masked like everything `read`
/// shows, since the reader may be on another device (D-07).
fn masked(text: &str) -> String {
    session_excerpt::mask(text)
}

fn project_value(project: &ScopeProject) -> Value {
    json!({
        "label": project.label,
        "root": project.root,
        "device_id": project.device_id,
    })
}

/// The title as the record screen names a session (`recordTitle`): the
/// operator's own title, else the main flow with summaries on (`[원문]` when
/// a `/goal` named it), else the agent's title, else its product's name.
fn title(
    manual: Option<&str>,
    main_flow: Option<(&str, bool)>,
    native: Option<&str>,
    agent: &str,
    summaries: bool,
) -> (String, &'static str) {
    if let Some(manual) = manual {
        return (masked(manual), RAW);
    }
    if summaries && let Some((flow, by_goal)) = main_flow {
        return (masked(flow), if by_goal { RAW } else { AI });
    }
    (
        native.map_or_else(|| crate::sidebar::provider_name(Some(agent)), masked),
        RAW,
    )
}

fn end_word(end: LabelEnd) -> &'static str {
    match end {
        LabelEnd::Working => "working",
        LabelEnd::Question => "question",
        LabelEnd::Blocked => "blocked",
        LabelEnd::Done => "done",
        LabelEnd::Waiting => "waiting",
        LabelEnd::Unfinished => "unfinished",
        LabelEnd::Limited => "limited",
    }
}

fn identity_value(session: &SessionIdentity, facts: &SessionFacts) -> Value {
    json!({
        "id": facts.agent_id,
        "session_id": session.session_id,
        "device_id": session.device_id,
        "agent": session.agent,
    })
}

fn list(
    store: &LinkStore,
    scope: &SessionScope,
    caller: &ScopeProject,
    members: &BTreeSet<SessionIdentity>,
    since_ms: Option<u64>,
    limit: Option<u32>,
    host: &dyn SessionHost,
) -> Result<Value, Refusal> {
    let period = since_ms.unwrap_or(LIST_SINCE_DEFAULT_MS);
    let limit = limit.unwrap_or(LIST_LIMIT_DEFAULT) as usize;
    let since = scope.now_unix_ms.saturating_sub(period);
    let page = store.session_page(members, since, limit).map_err(failed)?;
    let facts = host
        .facts(
            page.sessions
                .iter()
                .map(|row| row.session.clone())
                .collect(),
        )
        .ok_or_else(core_gone)?;
    let sessions = page
        .sessions
        .iter()
        .zip(
            facts
                .iter()
                .chain(std::iter::repeat(&SessionFacts::default())),
        )
        .map(|(row, facts)| listed_value(scope, row, facts))
        .collect::<Vec<_>>();
    Ok(json!({
        "verb": "list",
        "project": project_value(caller),
        "since_ms": period,
        "since_unix_ms": since,
        "now_unix_ms": scope.now_unix_ms,
        "limit": limit,
        "sessions": sessions,
        "more": page.more,
    }))
}

fn listed_value(scope: &SessionScope, row: &ListedSession, facts: &SessionFacts) -> Value {
    let (title, title_source) = title(
        row.manual_title.as_deref(),
        row.main_flow
            .as_ref()
            .map(|flow| (flow.name.as_str(), flow.named_by_goal)),
        row.native_title.as_deref(),
        &row.session.agent,
        scope.summaries,
    );
    json!({
        "session": identity_value(&row.session, facts),
        "agent_name": crate::sidebar::provider_name(Some(&row.session.agent)),
        "device": device_value(scope, &row.session.device_id),
        "title": title,
        "title_source": title_source,
        "state": state_value(facts, row.status.as_ref(), scope.summaries),
        "prs": row.prs,
        "requests": row.request_count,
        "last_activity_unix_ms": row.activity_unix_ms,
        "archived_at_unix_ms": facts.archived_at_unix_ms,
        "cwd": row.cwd,
        "file_gone": row.file_gone,
    })
}

/// A session's state: its live pane's while it works or asks, else the
/// record's last real turn (B1).
fn state_value(
    facts: &SessionFacts,
    status: Option<&super::ledger::RecordStatus>,
    summaries: bool,
) -> Value {
    if let Some(kind) = facts.live.as_ref().and_then(|live| live.kind) {
        return json!({"kind": kind, "source": "live"});
    }
    match status {
        Some(status) if summaries => {
            let (kind, line) = match &status.settled {
                Some(settled) => (
                    match settled.state {
                        super::ledger::SettledState::Merged => "merged",
                        super::ledger::SettledState::Closed => "closed",
                    },
                    None,
                ),
                None => (end_word(status.end), Some(status.line.as_str())),
            };
            json!({
                "kind": kind,
                "source": "record",
                "line": line.filter(|line| !line.is_empty()).map(masked),
                "line_source": AI,
                "at_unix_ms": status.at_unix_ms,
                "limited": status.limited,
                "prs": status.settled.as_ref().map(|settled| &settled.numbers),
            })
        }
        _ => json!({"kind": Value::Null, "source": "none"}),
    }
}

fn record(
    store: &LinkStore,
    scope: &SessionScope,
    members: &BTreeSet<SessionIdentity>,
    session: &SessionIdentity,
    host: &dyn SessionHost,
) -> Result<Value, Refusal> {
    let record = store.ledger_record(session).map_err(failed)?;
    let mut record = host.lay_pulls(record).ok_or_else(core_gone)?;
    // The record's tree and the scope's closure share one depth limit; a
    // child either missed is left out rather than shown outside the scope.
    record
        .children
        .retain(|child| members.contains(&child.session));
    for group in &mut record.pr_groups {
        group.name = masked(&group.name);
        for pull in &mut group.pulls {
            pull.title = masked(&pull.title);
            for holder in &mut pull.holders {
                *holder = masked(holder);
            }
        }
    }
    let mut asked = vec![record.session.clone()];
    asked.extend(
        record
            .children
            .iter()
            .take(CHILDREN_LIMIT)
            .map(|child| child.session.clone()),
    );
    let mut facts = host.facts(asked).ok_or_else(core_gone)?.into_iter();
    let own = facts.next().unwrap_or_default();
    let project = scope
        .projects
        .iter()
        .find(|project| {
            project.device_id == session.device_id
                && record.cwd.as_deref().is_some_and(|cwd| {
                    std::iter::once(&project.root)
                        .chain(&project.checkouts)
                        .any(|owner| super::within(cwd, owner))
                })
        })
        .map(project_value);
    let summaries = scope.summaries;
    let (title_text, title_source) = title(
        record.manual_title.as_deref(),
        main_flow(&record.flows).map(|flow| (flow.name.as_str(), named_by_goal(flow))),
        record.native_title.as_deref(),
        &session.agent,
        summaries,
    );
    let mut number = 0u32;
    let flow_count = record.flows.len();
    let flows = record
        .flows
        .iter()
        .enumerate()
        .map(|(index, flow)| {
            let requests = flow
                .requests
                .iter()
                .map(|request| request_value(request, &mut number, summaries))
                .collect::<Vec<_>>();
            json!({
                "name": summaries.then(|| masked(&flow.name)),
                "name_source": if named_by_goal(flow) { RAW } else { AI },
                "at_unix_ms": flow.at_unix_ms,
                "request_count": flow.request_count,
                "current": index + 1 == flow_count,
                "result": if summaries { flow.result.as_ref().map(result_value) } else { None },
                "requests": requests,
                "folded": false,
            })
        })
        .collect::<Vec<_>>();
    let live = own.live.clone().unwrap_or_default();
    let now = live.kind.map(|kind| {
        json!({
            "kind": kind,
            "request": live.request.as_deref().map(masked),
            "now": live.now.as_deref().map(masked),
        })
    });
    let children = record
        .children
        .iter()
        .take(CHILDREN_LIMIT)
        .zip(facts.chain(std::iter::repeat(SessionFacts::default())))
        .map(|(child, facts)| {
            let (title, title_source) = title(
                child.manual_title.as_deref(),
                main_flow(&child.flows).map(|flow| (flow.name.as_str(), named_by_goal(flow))),
                child.native_title.as_deref(),
                &child.session.agent,
                summaries,
            );
            json!({
                "session": identity_value(&child.session, &facts),
                "device": device_value(scope, &child.session.device_id),
                "title": title,
                "title_source": title_source,
                "state": state_value(&facts, child.status.as_ref(), summaries),
                "prs": child.outputs.iter().filter(|output| output.kind == "pr").count(),
                "archived_at_unix_ms": facts.archived_at_unix_ms,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "verb": "record",
        "session": identity_value(session, &own),
        "now_unix_ms": scope.now_unix_ms,
        "summaries": summaries,
        "head": {
            "title": title_text,
            "title_source": title_source,
            "device": device_value(scope, &session.device_id),
            "agent": session.agent,
            "agent_name": crate::sidebar::provider_name(Some(&session.agent)),
            "project": project,
            "checkout": record.cwd,
        },
        "archived_at_unix_ms": own.archived_at_unix_ms,
        "native_state": record.state,
        "state": state_value(&own, record.status.as_ref(), summaries),
        "now": now,
        "asks": if summaries {
            record.asks.iter().map(|ask| json!({"text": masked(&ask.text), "at_unix_ms": ask.at_unix_ms, "source": AI})).collect::<Vec<_>>()
        } else {
            Vec::new()
        },
        "flows": flows,
        "omitted_requests": record.omitted_requests,
        "folded_flows": 0,
        "pr_groups": record.pr_groups,
        "pr_omitted": record.pr_omitted,
        "artifacts": if summaries { record.artifacts.iter().map(|artifact| masked(artifact)).collect() } else { Vec::new() },
        "children": children,
        "children_omitted": record.children.len().saturating_sub(CHILDREN_LIMIT),
    }))
}

/// One request line: its number among the session's requests (letters are
/// not counted), its time and its words, summarized or as sent (B6).
fn request_value(request: &RecordedRequest, number: &mut u32, summaries: bool) -> Value {
    let letter = request.letter_from.is_some();
    let shown_number = (!letter).then(|| {
        *number += 1;
        *number
    });
    let (text, source) = match request.summary.as_deref() {
        Some(summary) if summaries && !letter => (masked(summary), AI),
        // Masked whole before the first line is cut, so a cut never splits
        // a secret past recognition.
        _ => (
            session_excerpt::cut(
                masked(&request.text).split('\n').next().unwrap_or_default(),
                RAW_REQUEST_CHARS,
            ),
            RAW,
        ),
    };
    json!({
        "number": shown_number,
        "at_unix_ms": request.at_unix_ms,
        "text": text,
        "source": source,
        "letter_from": request.letter_from,
        "answers": request.answers.as_deref().map(masked),
        "images": request.images,
        "count": request.count,
        "goal_command": request.goal_command,
    })
}

fn result_value(result: &FlowResult) -> Value {
    match result {
        FlowResult::Turn(turn) => json!({
            "kind": end_word(turn.end), "line": masked(&turn.line), "source": AI, "at_unix_ms": turn.at_unix_ms,
        }),
        FlowResult::Closing { line, at_unix_ms } => json!({
            "kind": "closing", "line": masked(line), "source": AI, "at_unix_ms": at_unix_ms,
        }),
        FlowResult::MovedOn { line, at_unix_ms } => json!({
            "kind": "moved_on", "line": masked(line), "source": AI, "at_unix_ms": at_unix_ms,
        }),
        FlowResult::Settled(settled) => json!({
            "kind": match settled.state {
                super::ledger::SettledState::Merged => "merged",
                super::ledger::SettledState::Closed => "closed",
            },
            "prs": settled.numbers,
            "source": RAW,
            "at_unix_ms": settled.at_unix_ms,
        }),
    }
}

fn read(
    store: &LinkStore,
    scope: &SessionScope,
    session: &SessionIdentity,
    request: Option<u32>,
    from: Option<u32>,
    events: Option<u32>,
) -> Result<Value, Refusal> {
    let file = store
        .session_file(session)
        .map_err(failed)?
        .ok_or_else(|| no("not_found", "Check the id with hide session list"))?;
    let agent = hide_session::Agent::from_kind(&session.agent).ok_or_else(|| {
        no(
            "session_unreadable",
            "Hide has no reader for this agent's sessions; read it with hide session record",
        )
    })?;
    let path = file
        .path
        .clone()
        .filter(|path| !path.is_empty())
        .ok_or_else(|| {
            no(
                "session_file_missing",
                "The session's file is gone; read what the record kept with hide session record",
            )
        })?;
    // A file whose path names a credential file, a secrets folder or Git's
    // own folder is never read (D-07), as no investigation read is.
    if path
        .split(['/', '\\'])
        .any(session_excerpt::credential_name)
    {
        return Err(no(
            "credential_path_refused",
            "The session's file path looks like a credential file; read it with hide session record",
        ));
    }
    if file.file_gone {
        return Err(no(
            "session_file_missing",
            "The session's file is gone; read what the record kept with hide session record",
        ));
    }
    let count = events.unwrap_or(READ_EVENTS_DEFAULT);
    let window = match (request, from) {
        (Some(number), _) => {
            // The session's own flows, as its record numbers them; its
            // children's requests are not loaded.
            let flows = store.ledger_flows(session).map_err(failed)?;
            let requests = flows
                .iter()
                .flat_map(|flow| &flow.requests)
                .filter(|request| request.letter_from.is_none())
                .collect::<Vec<_>>();
            let at = (number as usize)
                .checked_sub(1)
                .and_then(|index| requests.get(index))
                .map(|request| request.at_unix_ms)
                .ok_or_else(|| out_of_range("requests", requests.len()))?;
            SessionWindow::Around {
                at_unix_ms: at,
                count,
            }
        }
        // Event 0 is in no session: the read then only counts, and its end
        // names the range (B11).
        (None, Some(first)) => SessionWindow::From {
            from: first.checked_sub(1).unwrap_or(u32::MAX),
            count,
        },
        (None, None) => unreachable!("validated"),
    };
    let node = scope
        .nodes
        .iter()
        .find(|(device, _)| *device == session.device_id)
        .map(|(_, node)| Arc::clone(node))
        .ok_or_else(|| no("device_unavailable", "The device that holds this session is not connected; reconnect it and retry, or read it with hide session record"))?;
    let located = Located {
        agent,
        path,
        scope: hide_session::SessionReadScope {
            id: session.session_id.clone(),
            cwd: file.cwd.clone().unwrap_or_default(),
        },
    };
    let deadline = Instant::now() + READ_TIMEOUT;
    let read = session_excerpt::read(
        node.as_ref(),
        &located,
        window,
        Extent::Count { deadline },
        &|_, event| shown_event(event),
    )
    .map_err(read_refusal)?;
    if !read.reached {
        // The session reader's byte bound stopped the read before the
        // asked event; what it read is named, never shown as the answer.
        return Err(out_of_range("readable events", read.seen));
    }
    if read.kept.is_empty() {
        if read.complete && read.seen > 0 {
            return Err(out_of_range("events", read.seen));
        }
        return Err(no(
            "session_file_missing",
            "The session's file holds no events; read what the record kept with hide session record",
        ));
    }
    let first = read.kept.first().map_or(0, |(index, _)| index + 1);
    let last = read.kept.last().map_or(0, |(index, _)| index + 1);
    let next = (!read.complete || last < read.seen).then_some(last + 1);
    Ok(json!({
        "verb": "read",
        "session": {
            "session_id": session.session_id,
            "device_id": session.device_id,
            "agent": session.agent,
        },
        "request": request,
        "first": first,
        "last": last,
        "total": read.complete.then_some(read.seen),
        "next_from": next,
        "events": read.kept.into_iter().map(|(index, mut event)| {
            event["number"] = json!(index + 1);
            event
        }).collect::<Vec<_>>(),
    }))
}

/// One event as `read` shows it: redacted before it is cut, at most
/// [`session_excerpt::EVENT_CHARS`] characters and
/// [`READ_LINE_CHARS`] per line.
fn shown_event(event: &hide_session::ConversationEvent) -> Value {
    let (redacted, _) = session_excerpt::egress(&event.text, usize::MAX);
    let mut cut_any = false;
    let mut lines = Vec::new();
    for line in redacted.lines() {
        let (shown, cut) = session_excerpt::cut_marked(line, READ_LINE_CHARS);
        cut_any |= cut;
        lines.push(shown);
    }
    let (text, cut) = session_excerpt::cut_marked(&lines.join("\n"), session_excerpt::EVENT_CHARS);
    cut_any |= cut;
    json!({
        "at_unix_ms": event.at_unix_ms,
        "role": event.role,
        "kind": event.kind.as_str(),
        "text": text,
        "images": event.images,
        "cut": cut_any,
    })
}

fn out_of_range(what: &'static str, count: usize) -> Refusal {
    // The valid range is the refusal's detail, which the daemon puts in its
    // next action (B11).
    let range = match (what, count) {
        ("requests", 0) => "the session has no recorded requests".to_owned(),
        ("requests", _) => format!("requests 1-{count}"),
        ("readable events", _) => format!(
            "events 1-{count} are readable; the rest is past the read bound, so read it with hide session record"
        ),
        (_, _) => format!("events 1-{count}"),
    };
    (
        crate::coordination::refusal("out_of_range", &range),
        "Pass a number in the session's range",
    )
}

fn read_refusal(failure: ReadFailure) -> Refusal {
    use hide_node_link::{ErrorCode, LinkError};
    match failure {
        ReadFailure::Deadline => no(
            "read_timeout",
            "The device did not answer within 60 s; retry, or read it with hide session record",
        ),
        // The call may have reached the node, but no answer came back: a
        // link that dropped, or an answer that could not be read.
        ReadFailure::Link(LinkError::Unknown(_)) => {
            crate::diagnostic!(json!({"component": "session_cli", "kind": "read.unanswered"}));
            no(
                "read_timeout",
                "The device did not answer; retry, or read it with hide session record",
            )
        }
        ReadFailure::Link(LinkError::NotConnected(_)) => no(
            "device_unavailable",
            "The device that holds this session is not connected; reconnect it and retry",
        ),
        ReadFailure::Link(LinkError::Busy) => {
            no("read_timeout", "The device is busy; retry in a moment")
        }
        // The node names a file that is no longer there by its reason; its
        // code is the generic `io` (`hide-host` `session_file`).
        ReadFailure::Link(LinkError::Refused(error)) if error.message == "session_file_missing" => {
            no(
                "session_file_missing",
                "The session's file is gone; read what the record kept with hide session record",
            )
        }
        ReadFailure::Link(LinkError::Refused(error)) => match error.code {
            ErrorCode::NotFound => no(
                "session_file_missing",
                "The session's file is gone; read what the record kept with hide session record",
            ),
            ErrorCode::Unsupported => no(
                "session_unreadable",
                "The device has no reader for this agent's sessions; update its helper, or read it with hide session record",
            ),
            _ => {
                crate::diagnostic!(
                    json!({"component": "session_cli", "kind": "read.refused", "code": error.code.as_str()})
                );
                no(
                    "session_unreadable",
                    "The device could not read the session; read it with hide session record",
                )
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::ledger::{LedgerUpdate, RequestSummary, WorkAuthor};
    use crate::links::{ParentFact, ProjectFacts};
    use hide_node_link::link::LinkAnswer;
    use hide_node_link::protocol::Call;
    use hide_node_link::sessions::ReaderFeatures;
    use hide_node_link::{ErrorCode, HostError, LinkError};

    const NOW: u64 = 1_790_000_000_000;
    const HOUR: u64 = 60 * 60 * 1000;
    const CLAUDE: &str = "claude";

    fn identity(device: &str, id: &str) -> SessionIdentity {
        SessionIdentity {
            device_id: device.into(),
            agent: CLAUDE.into(),
            session_id: id.into(),
        }
    }

    fn project(key: &str, device: &str, root: &str, repository: Option<&str>) -> ProjectFacts {
        ProjectFacts {
            key: key.into(),
            device_id: device.into(),
            workspace_id: format!("ws-{key}"),
            root: root.into(),
            repository: repository.map(Into::into),
            repository_id: None,
            connection_epoch: 0,
            worktrees: Vec::new(),
            prs: Vec::new(),
            prs_read: true,
            outputs: Vec::new(),
        }
    }

    fn requests(
        store: &mut LinkStore,
        session: &SessionIdentity,
        cwd: &str,
        path: &str,
        at: &[u64],
    ) {
        store
            .apply_ledger_update(&LedgerUpdate::Requests {
                session: session.clone(),
                cwd: cwd.into(),
                path: Some(path.into()),
                requests: at
                    .iter()
                    .enumerate()
                    .map(|(offset, at)| RecordedRequest {
                        offset: offset as u64,
                        at_unix_ms: *at,
                        text: format!("request {offset} {}", "말".repeat(200)),
                        author: WorkAuthor::Operator,
                        count: 1,
                        ..RecordedRequest::default()
                    })
                    .collect(),
            })
            .unwrap();
    }

    /// `app` here and its clone on device `mini` share `acme/app` by their
    /// `origin`, though GitHub was never read for the clone (no name in the
    /// record, as on the operator's mini); `lib` is another Project. `s-app` worked in `app` and delegated `s-child`,
    /// which ran outside every Project's folder.
    fn fixture() -> (tempfile::TempDir, SessionScope) {
        let state = tempfile::tempdir().unwrap();
        let path = state.path().join("links.sqlite3");
        let (mut store, _) = LinkStore::open(&path).unwrap();
        for facts in [
            project("app", "local", "/work/app", Some("acme/app")),
            project("app-mini", "mini", "/home/u/app", None),
            project("lib", "local", "/work/lib", Some("acme/lib")),
        ] {
            store.apply_project(&facts, NOW).unwrap();
        }
        let base = NOW - HOUR;
        requests(
            &mut store,
            &identity("local", "s-app"),
            "/work/app/feat",
            "/home/u/.claude/projects/app/s-app.jsonl",
            &[base, base + 10_000, base + 20_000],
        );
        store
            .apply_ledger_update(&LedgerUpdate::Summaries {
                session: identity("local", "s-app"),
                summaries: vec![RequestSummary {
                    offset: 0,
                    summary: "로그인 버그 고치기".into(),
                    answers: None,
                }],
            })
            .unwrap();
        requests(
            &mut store,
            &identity("mini", "s-mini"),
            "/home/u/app",
            "/home/u/.claude/projects/app/s-mini.jsonl",
            &[base + 30_000],
        );
        requests(
            &mut store,
            &identity("local", "s-lib"),
            "/work/lib",
            "/home/u/.claude/projects/lib/s-lib.jsonl",
            &[base + 40_000],
        );
        requests(
            &mut store,
            &identity("local", "s-child"),
            "/tmp/elsewhere",
            "/home/u/.claude/projects/tmp/s-child.jsonl",
            &[base + 50_000],
        );
        requests(
            &mut store,
            &identity("local", "s-secret"),
            "/work/app",
            "/home/u/secrets/s-secret.jsonl",
            &[base - 2 * HOUR],
        );
        store
            .apply_parents(&[ParentFact {
                device_id: "local".into(),
                agent: CLAUDE.into(),
                session_id: "s-child".into(),
                parent_device_id: "local".into(),
                parent_agent: CLAUDE.into(),
                parent_session_id: "s-app".into(),
                parent_name: "lead".into(),
            }])
            .unwrap();
        let scope_project = |key: &str, device: &str, root: &str| ScopeProject {
            key: key.into(),
            workspace_id: format!("ws-{key}"),
            label: key.into(),
            root: root.into(),
            device_id: device.into(),
            checkouts: vec![root.into()],
        };
        let node: Arc<dyn NodeLink> = Arc::new(Shelf);
        let scope = SessionScope {
            store: path,
            local_device: "local".into(),
            caller: "app".into(),
            projects: vec![
                scope_project("app", "local", "/work/app"),
                scope_project("app-mini", "mini", "/home/u/app"),
                scope_project("lib", "local", "/work/lib"),
            ],
            repositories: BTreeMap::from([
                ("app".to_owned(), "acme/app".to_owned()),
                ("app-mini".to_owned(), "Acme/App".to_owned()),
                ("lib".to_owned(), "acme/lib".to_owned()),
            ]),
            devices: vec![
                DeviceFacts {
                    id: "local".into(),
                    label: None,
                    local: true,
                    unreachable_since_unix_ms: None,
                },
                DeviceFacts {
                    id: "mini".into(),
                    label: Some("mini".into()),
                    local: false,
                    unreachable_since_unix_ms: Some(NOW - 5 * 60_000),
                },
            ],
            nodes: vec![("local".into(), node)],
            summaries: true,
            now_unix_ms: NOW,
        };
        (state, scope)
    }

    #[derive(Default)]
    struct Host {
        archived: Option<SessionIdentity>,
    }

    impl SessionHost for Host {
        fn agent_session(&self, id: &str) -> Option<SessionIdentity> {
            (id == "agent-7").then(|| identity("local", "s-app"))
        }
        fn facts(&self, sessions: Vec<SessionIdentity>) -> Option<Vec<SessionFacts>> {
            Some(
                sessions
                    .into_iter()
                    .map(|session| SessionFacts {
                        agent_id: (session.session_id == "s-app").then(|| "agent-7".to_owned()),
                        live: None,
                        archived_at_unix_ms: (self.archived.as_ref() == Some(&session))
                            .then_some(NOW - 2 * HOUR),
                    })
                    .collect(),
            )
        }
        fn lay_pulls(&self, record: SessionRecord) -> Option<SessionRecord> {
            Some(record)
        }
    }

    /// A node holding one conversation of 30 events, one a minute from an
    /// hour ago; the 3rd carries a credential.
    struct Shelf;

    impl NodeLink for Shelf {
        fn call(&self, call: Call, _: Duration) -> Result<LinkAnswer, LinkError> {
            let Call::SessionConversation { path, .. } = call else {
                panic!("only conversation reads");
            };
            if path.ends_with("s-child.jsonl") {
                // As `hide-host` answers a file that is no longer there.
                return Err(LinkError::Refused(HostError::new(
                    ErrorCode::Io,
                    "session_file_missing",
                )));
            }
            let events: Vec<hide_session::ConversationEvent> = (0..30)
                .map(|n| {
                    let text = if n == 2 {
                        "export API_KEY=sk-livevalue1234567890abcdef".to_owned()
                    } else {
                        format!("event {n} {}", "x".repeat(500))
                    };
                    hide_session::ConversationEvent::new(
                        "user",
                        hide_session::EventKind::Human,
                        NOW - HOUR + n * 10_000,
                        text,
                    )
                })
                .collect();
            let read = hide_session::ConversationRead {
                event_offsets: (0..events.len() as u64).collect(),
                events,
                checkpoint: Default::default(),
                has_more: false,
                rescanned: false,
                read_bytes: 4_096,
                skipped_reasons: Default::default(),
            };
            Ok(LinkAnswer::Parsed(serde_json::to_value(read).unwrap()))
        }

        fn reader_features(&self) -> Option<&ReaderFeatures> {
            static FEATURES: std::sync::LazyLock<ReaderFeatures> =
                std::sync::LazyLock::new(ReaderFeatures::implemented);
            Some(&FEATURES)
        }
    }

    fn run_with(scope: &SessionScope, command: SessionCommand) -> Result<Value, Refusal> {
        run(scope, &command, &Host::default())
    }

    fn list(scope: &SessionScope, since_ms: Option<u64>, limit: Option<u32>) -> Value {
        run_with(scope, SessionCommand::List { since_ms, limit }).unwrap()
    }

    fn ids(answer: &Value) -> Vec<String> {
        answer["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["session"]["session_id"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn a_list_holds_the_projects_sessions_its_devices_clones_and_their_descendants_newest_first() {
        let (_state, scope) = fixture();
        let answer = list(&scope, None, None);
        assert_eq!(ids(&answer), ["s-child", "s-mini", "s-app", "s-secret"]);
        assert_eq!(answer["more"], 0);
        let app = &answer["sessions"][2];
        assert_eq!(app["session"]["id"], "agent-7");
        assert_eq!(app["requests"], 3);
        let mini = &answer["sessions"][1];
        assert_eq!(
            mini["device"]["disconnected_since_unix_ms"],
            NOW - 5 * 60_000,
            "a disconnected device's session keeps its last value with its age"
        );
    }

    #[test]
    fn a_list_bounds_its_period_and_its_length_and_counts_the_rest() {
        let (_state, scope) = fixture();
        let short = list(&scope, Some(HOUR), Some(2));
        assert_eq!(ids(&short), ["s-child", "s-mini"]);
        assert_eq!(short["more"], 1, "s-app is in the period, s-secret is not");
        let empty = list(&scope, Some(60_000), None);
        assert_eq!(empty["sessions"], json!([]));
        assert_eq!(empty["since_ms"], 60_000);
        for limit in [0, LIST_LIMIT_MAX + 1] {
            assert_eq!(
                run_with(
                    &scope,
                    SessionCommand::List {
                        since_ms: None,
                        limit: Some(limit)
                    }
                )
                .unwrap_err()
                .0,
                "invalid_request"
            );
        }
    }

    #[test]
    fn a_device_clone_joins_by_its_origin_and_an_unknown_repository_stays_alone() {
        let (_state, scope) = fixture();
        let listed = |scope: &SessionScope| ids(&list(scope, None, None));
        // GitHub never named the clone; its `origin` does.
        assert!(listed(&scope).contains(&"s-mini".to_owned()));
        let mut unknown = scope.clone();
        unknown.repositories.remove("app");
        assert_eq!(listed(&unknown), ["s-child", "s-app", "s-secret"]);
        let mut other = scope.clone();
        other
            .repositories
            .insert("app-mini".into(), "acme/fork".into());
        assert!(!listed(&other).contains(&"s-mini".to_owned()));
    }

    #[test]
    fn a_session_outside_the_scope_or_unknown_is_refused() {
        let (_state, scope) = fixture();
        let refused = |id: &str| {
            run_with(&scope, SessionCommand::Record { id: id.into() })
                .unwrap_err()
                .0
        };
        assert_eq!(refused("s-lib"), "other_project");
        assert_eq!(refused("s-nowhere"), "not_found");
        assert_eq!(refused("agent-9"), "not_found");
        let mut outside = scope.clone();
        outside.caller = String::new();
        assert_eq!(
            run_with(&outside, SessionCommand::Record { id: "s-app".into() })
                .unwrap_err()
                .0,
            "workspace_unavailable"
        );
    }

    #[test]
    fn a_record_numbers_its_requests_and_marks_each_summary_or_raw() {
        let (_state, scope) = fixture();
        let answer = run_with(
            &scope,
            SessionCommand::Record {
                id: "agent-7".into(),
            },
        )
        .unwrap();
        assert_eq!(answer["session"]["session_id"], "s-app");
        let requests = answer["flows"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|flow| flow["requests"].as_array().unwrap().clone())
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0]["number"], 1);
        assert_eq!(requests[0]["text"], "로그인 버그 고치기");
        assert_eq!(requests[0]["source"], "ai_summary");
        assert_eq!(requests[1]["number"], 2);
        assert_eq!(requests[1]["source"], "raw");
        let raw = requests[1]["text"].as_str().unwrap();
        assert_eq!(raw.chars().count(), RAW_REQUEST_CHARS + 1, "{raw}");
        assert!(raw.ends_with('…'));
        assert_eq!(answer["children"][0]["session"]["session_id"], "s-child");

        let mut off = scope.clone();
        off.summaries = false;
        let answer = run_with(&off, SessionCommand::Record { id: "s-app".into() }).unwrap();
        let first = &answer["flows"][0]["requests"][0];
        assert_eq!(
            first["source"], "raw",
            "summaries off shows the words as sent"
        );
        assert_eq!(answer["flows"][0]["name"], Value::Null);
    }

    #[test]
    fn an_archived_session_is_listed_and_recorded_with_its_archive_time() {
        let (_state, scope) = fixture();
        let host = Host {
            archived: Some(identity("local", "s-app")),
        };
        let answer = run(
            &scope,
            &SessionCommand::List {
                since_ms: None,
                limit: None,
            },
            &host,
        )
        .unwrap();
        assert_eq!(answer["sessions"][2]["archived_at_unix_ms"], NOW - 2 * HOUR);
        let record = run(
            &scope,
            &SessionCommand::Record { id: "s-app".into() },
            &host,
        )
        .unwrap();
        assert_eq!(record["archived_at_unix_ms"], NOW - 2 * HOUR);
    }

    fn read(
        scope: &SessionScope,
        id: &str,
        request: Option<u32>,
        from: Option<u32>,
    ) -> Result<Value, Refusal> {
        run_with(
            scope,
            SessionCommand::Read {
                id: id.into(),
                request,
                from,
                events: None,
            },
        )
    }

    #[test]
    fn a_read_around_a_request_shows_numbered_bounded_redacted_events() {
        let (_state, scope) = fixture();
        // Request 1 was sent at the first event's time.
        let answer = read(&scope, "s-app", Some(1), None).unwrap();
        assert_eq!(answer["first"], 1);
        assert_eq!(answer["last"], READ_EVENTS_DEFAULT);
        assert_eq!(answer["total"], 30);
        assert_eq!(answer["next_from"], READ_EVENTS_DEFAULT + 1);
        let events = answer["events"].as_array().unwrap();
        assert_eq!(events.len(), READ_EVENTS_DEFAULT as usize);
        let secret = events[2]["text"].as_str().unwrap();
        assert!(!secret.contains("sk-livevalue"), "{secret}");
        let long = events[0]["text"].as_str().unwrap();
        assert!(long.chars().count() <= READ_LINE_CHARS + 1);
        assert_eq!(events[0]["cut"], true);

        let tail = read(&scope, "s-app", None, Some(25)).unwrap();
        assert_eq!(
            (tail["first"].clone(), tail["last"].clone()),
            (json!(25), json!(30))
        );
        assert_eq!(
            tail["next_from"],
            Value::Null,
            "the last event ends the read"
        );
    }

    #[test]
    fn a_read_past_the_session_names_the_range() {
        let (_state, scope) = fixture();
        let (code, next) = read(&scope, "s-app", Some(4), None).unwrap_err();
        assert_eq!(
            crate::coordination::split_refusal(&code),
            ("out_of_range", Some("requests 1-3"))
        );
        assert!(!next.is_empty());
        let (code, _) = read(&scope, "s-app", None, Some(31)).unwrap_err();
        assert_eq!(
            crate::coordination::split_refusal(&code),
            ("out_of_range", Some("events 1-30"))
        );
    }

    #[test]
    fn a_read_refuses_a_credential_path_a_gone_file_and_a_disconnected_device() {
        let (_state, scope) = fixture();
        let code = |id: &str| read(&scope, id, None, Some(1)).unwrap_err().0;
        assert_eq!(code("s-secret"), "credential_path_refused");
        assert_eq!(code("s-child"), "session_file_missing");
        assert_eq!(code("s-mini"), "device_unavailable");
    }

    #[test]
    fn a_session_with_no_reader_is_listed_but_not_read() {
        let (state, scope) = fixture();
        let (mut store, _) = LinkStore::open(&state.path().join("links.sqlite3")).unwrap();
        let unknown = SessionIdentity {
            device_id: "local".into(),
            agent: "mystery".into(),
            session_id: "s-mystery".into(),
        };
        requests(
            &mut store,
            &unknown,
            "/work/app",
            "/home/u/m.jsonl",
            &[NOW - 1_000],
        );
        drop(store);
        assert!(ids(&list(&scope, None, None)).contains(&"s-mystery".to_owned()));
        assert_eq!(
            read(&scope, "s-mystery", None, Some(1)).unwrap_err().0,
            "session_unreadable"
        );
    }

    fn add_session(
        scope: &SessionScope,
        session: &SessionIdentity,
        cwd: &str,
        texts: &[(u64, &str)],
    ) {
        let (mut store, _) = LinkStore::open(&scope.store).unwrap();
        store
            .apply_ledger_update(&LedgerUpdate::Requests {
                session: session.clone(),
                cwd: cwd.into(),
                path: Some(format!(
                    "/home/u/.claude/projects/x/{}.jsonl",
                    session.session_id
                )),
                requests: texts
                    .iter()
                    .enumerate()
                    .map(|(offset, (at, text))| RecordedRequest {
                        offset: offset as u64,
                        at_unix_ms: *at,
                        text: (*text).to_owned(),
                        author: WorkAuthor::Operator,
                        count: 1,
                        ..RecordedRequest::default()
                    })
                    .collect(),
            })
            .unwrap();
    }

    #[test]
    fn a_record_masks_a_secret_named_like_an_environment_variable() {
        let (_state, mut scope) = fixture();
        scope.summaries = false;
        add_session(
            &scope,
            &identity("local", "s-leak"),
            "/work/app",
            &[(NOW - 1_000, "DB_PASSWORD=hunter2hunter2 then fix the login")],
        );
        let (mut store, _) = LinkStore::open(&scope.store).unwrap();
        store
            .apply_ledger_update(&LedgerUpdate::Summaries {
                session: identity("local", "s-leak"),
                summaries: vec![RequestSummary {
                    offset: 0,
                    summary: "로그인 고치기".into(),
                    answers: Some("PRIVATE_KEY=abcdefgh12345678 에 답함".into()),
                }],
            })
            .unwrap();
        drop(store);
        let answer = run_with(
            &scope,
            SessionCommand::Record {
                id: "s-leak".into(),
            },
        )
        .unwrap();
        let text = answer.to_string();
        assert!(!text.contains("hunter2"), "{text}");
        assert!(!text.contains("abcdefgh12345678"), "{text}");
        assert!(text.contains("fix the login"), "{text}");
    }

    #[test]
    fn a_list_counts_repeated_requests_as_the_record_numbers_them() {
        let (_state, scope) = fixture();
        add_session(
            &scope,
            &identity("local", "s-again"),
            "/work/app",
            &[
                (NOW - 50_000, "계속"),
                (NOW - 40_000, "계속"),
                (NOW - 30_000, "계속"),
                (NOW - 10_000, "다른 일"),
            ],
        );
        let listed = list(&scope, None, None);
        let row = listed["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["session"]["session_id"] == "s-again")
            .unwrap()
            .clone();
        assert_eq!(row["requests"], 2);
        let record = run_with(
            &scope,
            SessionCommand::Record {
                id: "s-again".into(),
            },
        )
        .unwrap();
        let numbers: Vec<Value> = record["flows"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|flow| flow["requests"].as_array().unwrap().clone())
            .map(|request| request["number"].clone())
            .collect();
        assert_eq!(numbers, [json!(1), json!(2)]);
        let (code, _) = read(&scope, "s-again", Some(3), None).unwrap_err();
        assert_eq!(
            crate::coordination::split_refusal(&code),
            ("out_of_range", Some("requests 1-2"))
        );
    }

    #[test]
    fn a_project_nested_in_the_callers_keeps_its_own_sessions() {
        let (state, scope) = fixture();
        let (mut store, _) = LinkStore::open(&state.path().join("links.sqlite3")).unwrap();
        store
            .apply_project(
                &project("sub", "local", "/work/app/vendor/sub", Some("acme/sub")),
                NOW,
            )
            .unwrap();
        drop(store);
        add_session(
            &scope,
            &identity("local", "s-sub"),
            "/work/app/vendor/sub/src",
            &[(NOW - 1_000, "inner work")],
        );
        add_session(
            &scope,
            &identity("local", "s-vendor"),
            "/work/app/vendor",
            &[(NOW - 1_000, "outer work")],
        );
        let ids = ids(&list(&scope, None, None));
        assert!(!ids.contains(&"s-sub".to_owned()), "{ids:?}");
        assert!(ids.contains(&"s-vendor".to_owned()), "{ids:?}");
    }

    #[test]
    fn a_zero_number_is_out_of_range_with_the_range() {
        let (_state, scope) = fixture();
        let (code, _) = read(&scope, "s-app", Some(0), None).unwrap_err();
        assert_eq!(
            crate::coordination::split_refusal(&code),
            ("out_of_range", Some("requests 1-3"))
        );
        let (code, _) = read(&scope, "s-app", None, Some(0)).unwrap_err();
        assert_eq!(
            crate::coordination::split_refusal(&code),
            ("out_of_range", Some("events 1-30"))
        );
    }

    /// A node whose one page is already past the session reader's byte
    /// bound, so a read stops after it with more to come.
    struct Huge;

    impl NodeLink for Huge {
        fn call(&self, call: Call, timeout: Duration) -> Result<LinkAnswer, LinkError> {
            let answer = Shelf.call(call, timeout)?;
            let LinkAnswer::Parsed(mut page) = answer else {
                unreachable!("the shelf answers parsed pages");
            };
            page["has_more"] = json!(true);
            page["read_bytes"] = json!(hide_session::SESSION_READ_LIMIT_BYTES + 1);
            Ok(LinkAnswer::Parsed(page))
        }

        fn reader_features(&self) -> Option<&ReaderFeatures> {
            Shelf.reader_features()
        }
    }

    #[test]
    fn a_read_past_the_read_bound_names_the_readable_part_and_shows_nothing_else() {
        let (_state, mut scope) = fixture();
        scope.nodes = vec![("local".into(), Arc::new(Huge) as Arc<dyn NodeLink>)];
        let (code, _) = read(&scope, "s-app", None, Some(31)).unwrap_err();
        let (code, detail) = crate::coordination::split_refusal(&code);
        assert_eq!(code, "out_of_range");
        assert!(
            detail.unwrap().starts_with("events 1-30 are readable"),
            "{detail:?}"
        );
        let answer = read(&scope, "s-app", None, Some(1)).unwrap();
        assert_eq!(answer["total"], Value::Null, "the end was not reached");
        assert_eq!(answer["next_from"], READ_EVENTS_DEFAULT + 1);
    }
}
