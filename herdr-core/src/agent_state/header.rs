//! Quiet pane identity and one operator-facing band, from existing core facts.
//! No clock, I/O, delivery authority or terminal geometry lives here.
use crate::model::{PaneSnapshot, SidebarAgentSnapshot, TerminalPaneSnapshot, WorkspaceSnapshot};
use serde::Serialize;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Header {
    pub band: Option<Band>,
    pub working: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Band {
    pub kind: String,
    pub tone: &'static str,
    pub reason: Option<String>,
    pub since_unix_ms: Option<u64>,
    pub action: Option<Action>,
    pub more: usize,
    pub exit_code: Option<i32>,
    /// A raised descendant's band (PRD D-43): the lead ask, whose verb, what,
    /// who and path the band draws; `more` counts the others.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raised: Option<super::escalation::RaisedAsk>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facts: Option<ReasonFacts>,
}

/// Facts already read by the core; a generated progress sentence cannot stand
/// in for them.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReasonFacts {
    PullRequest {
        checks: crate::model::PullRequestChecks,
        review: Option<crate::model::ReviewDecision>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    Pr {
        workspace_id: String,
        url: String,
        number: u32,
        checks: crate::model::PullRequestChecks,
        tone: &'static str,
    },
    Child {
        pane_id: String,
        label: String,
    },
}

/// Why a device's helper link is away, when that is why a pane of the device
/// is unavailable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LinkWait {
    /// The core tries the link again by itself.
    Returns,
    /// The operator has not allowed the helper, or withdrew the consent.
    NeedsConsent,
}

/// A pane of a device whose helper link is away: a band that says the pane is
/// unavailable says what the link waits for, so the pane no longer reads as a
/// failure the operator could do something about, or as one nothing can fix.
pub(crate) fn waiting_on_link(mut header: Header, wait: LinkWait) -> Header {
    if let Some(band) = header
        .band
        .as_mut()
        .filter(|band| band.kind == "unavailable")
    {
        match wait {
            LinkWait::Returns => {
                band.kind = "reconnecting".to_owned();
                band.tone = "muted";
            }
            LinkWait::NeedsConsent => {
                band.kind = "helper_not_allowed".to_owned();
                band.tone = "warning";
            }
        }
    }
    header
}

pub(crate) fn of(
    pane: &PaneSnapshot,
    agent: Option<&SidebarAgentSnapshot>,
    transport: Option<&TerminalPaneSnapshot>,
    workspace: &WorkspaceSnapshot,
    offline: Option<&str>,
) -> Header {
    let tag = agent.map(task_kind);
    let links = agent
        .and_then(|agent| agent.request.as_ref())
        .map(|request| &request.pull_requests);
    let live = |pull: &&crate::request_view::AgentPullRequestSnapshot| {
        pull.live && !pull.badge.is_settled()
    };
    let action = |pull: &crate::request_view::AgentPullRequestSnapshot| Action::Pr {
        workspace_id: workspace.id.clone(),
        url: pull.url.clone(),
        number: pull.number,
        checks: pull.checks,
        tone: workspace
            .pull_requests
            .iter()
            .find(|pr| pr.url == pull.url)
            .map_or("pr", pr_tone),
    };
    let duty = links.and_then(|links| {
        links.iter().filter(live).find(|pull| {
            use crate::model::PullRequestChecks;
            match tag {
                Some(Tag::Fix) => pull.duty && pull.checks == PullRequestChecks::Failed,
                Some(Tag::Review) => {
                    pull.duty
                        && matches!(
                            pull.checks,
                            PullRequestChecks::Passing
                                | PullRequestChecks::None
                                | PullRequestChecks::Unknown
                        )
                        && pull.state() == crate::model::PrState::Pending
                }
                Some(Tag::Merge) => pull.duty && pull.state() == crate::model::PrState::Mergeable,
                _ => false,
            }
        })
    });
    let band = |kind: &str, tone, reason, since_unix_ms, action, more, exit_code| {
        Some(Band {
            kind: kind.to_owned(),
            tone,
            reason,
            since_unix_ms,
            action,
            more,
            exit_code,
            raised: None,
            facts: None,
        })
    };
    let unavailable = if let Some(name) = offline {
        band(
            "device_offline",
            "muted",
            Some(name.to_owned()),
            None,
            None,
            0,
            None,
        )
    } else if pane.failed_terminal_id.is_some() {
        band("restore_failed", "error", None, None, None, 0, None)
    } else if let Some(sleep) = &pane.sleep {
        band(
            &sleep.state,
            if sleep.state == "failed" {
                "error"
            } else {
                "muted"
            },
            sleep.reason.clone(),
            Some(sleep.since_unix_ms),
            None,
            0,
            None,
        )
    } else if let Some(transport) = transport {
        if transport.scroll_held_elsewhere {
            band("controlled_elsewhere", "muted", None, None, None, 0, None)
        } else if transport.closed || transport.transport_state == "ended" {
            let failed = transport.exit_code.is_some_and(|code| code != 0);
            band(
                if failed { "exit" } else { "terminated" },
                if failed { "error" } else { "muted" },
                None,
                None,
                None,
                0,
                transport.exit_code,
            )
        } else {
            match transport.transport_state.as_str() {
                "released" => band("disconnected", "muted", None, None, None, 0, None),
                "closing" => band("closing", "muted", None, None, None, 0, None),
                "unavailable" => band("unavailable", "error", None, None, None, 0, None),
                "starting" | "connecting" | "reconnecting" => {
                    band("starting", "muted", None, None, None, 0, None)
                }
                _ => None,
            }
        }
    } else {
        None
    };
    if unavailable.is_some() {
        return Header {
            band: unavailable,
            working: false,
        };
    }
    let Some(agent) = agent else {
        return Header::default();
    };
    let demand = matches!(tag, Some(Tag::Approval | Tag::Answer | Tag::Blocked));
    if !demand && let Some(lead) = agent.raised.first() {
        return Header {
            working: false,
            band: band(
                "raised",
                "warning",
                None,
                lead.since_unix_ms,
                Some(Action::Child {
                    pane_id: lead.open_pane_id.clone(),
                    label: lead.title.clone(),
                }),
                agent.raised.len() - 1,
                None,
            )
            .map(|mut band| {
                band.raised = Some(lead.clone());
                band
            }),
        };
    }
    let (kind, tone, action) = match tag {
        Some(Tag::Approval) => ("approval", "warning", None),
        Some(Tag::Answer) => ("answer", "warning", None),
        Some(Tag::Blocked) => ("blocked", "warning", None),
        Some(Tag::Fix) => ("fix", "error", duty.map(action)),
        Some(Tag::Review) => {
            let action = duty.map(action);
            ("review", pull_tone(&action), action)
        }
        Some(Tag::Merge) => {
            let action = duty.map(action);
            ("merge", pull_tone(&action), action)
        }
        Some(Tag::Stopped) => ("stopped", "warning", None),
        Some(Tag::Result) => ("result", "success", None),
        _ => {
            return Header {
                band: None,
                working: agent.state.verb == super::RequestVerb::Working,
            };
        }
    };
    Header {
        working: false,
        band: band(
            kind,
            tone,
            // An Answer says what the row's ask says (`detail`), so a native
            // question reads as itself here too, not as the label's line
            // about the work around it. An Approval has no command the core
            // read, and that line is not one, so it carries no reason.
            match tag {
                Some(Tag::Answer) => agent.detail.clone(),
                Some(Tag::Approval) => None,
                _ => agent
                    .request
                    .as_ref()
                    .and_then(|request| request.line.clone()),
            },
            agent.state.request_since,
            action,
            if demand { agent.raised.len() } else { 0 },
            None,
        )
        .map(|mut band| {
            band.facts = match tag {
                Some(Tag::Fix | Tag::Review | Tag::Merge) => {
                    duty.map(|pull| ReasonFacts::PullRequest {
                        checks: pull.checks,
                        review: pull.review,
                    })
                }
                _ => None,
            };
            if band.facts.is_some() {
                band.reason = None;
            }
            band
        }),
    }
}

/// The band a row's own task takes, from its request verb (docs/status-model.md,
/// Quiet pane headers).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tag {
    Answer,
    Approval,
    Blocked,
    Fix,
    Review,
    Merge,
    Stopped,
    Result,
    Quiet,
}

fn task_kind(agent: &SidebarAgentSnapshot) -> Tag {
    use super::RequestVerb;
    use super::escalation::Verb;
    match agent.state.verb {
        RequestVerb::Answer => match super::turn::demand_verb(agent) {
            Some(Verb::Approval) => Tag::Approval,
            _ => Tag::Answer,
        },
        RequestVerb::Blocked => Tag::Blocked,
        RequestVerb::Fix => Tag::Fix,
        RequestVerb::Review => {
            let mut duties = agent
                .request
                .iter()
                .flat_map(|request| &request.pull_requests)
                .filter(|pull| pull.live && pull.duty && !pull.badge.is_settled())
                .peekable();
            if duties.peek().is_some()
                && duties.all(|pull| pull.state() == crate::model::PrState::Mergeable)
            {
                Tag::Merge
            } else {
                Tag::Review
            }
        }
        RequestVerb::Stopped => Tag::Stopped,
        RequestVerb::Result => Tag::Result,
        RequestVerb::Working | RequestVerb::Waiting | RequestVerb::Idle => Tag::Quiet,
    }
}

fn pull_tone(pull: &Option<Action>) -> &'static str {
    match pull {
        Some(Action::Pr { tone, .. }) => tone,
        _ => "pr",
    }
}

/// The band's colour is the pull request's one state ([`crate::model::PrState::of`]).
fn pr_tone(pr: &crate::model::PullRequestSnapshot) -> &'static str {
    use crate::model::PrState;
    match pr.state() {
        PrState::Failed => "error",
        PrState::Mergeable => "success",
        PrState::Pending | PrState::Draft => "muted",
        PrState::Merged | PrState::Closed => "pr",
    }
}
