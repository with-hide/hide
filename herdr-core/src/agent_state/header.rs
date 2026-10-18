//! Quiet pane identity, why a pane's terminal is not live, and what waits on
//! the operator in it, from existing core facts.
//! No clock, I/O, delivery authority or terminal geometry lives here.
use crate::model::{PaneSnapshot, SidebarAgentSnapshot, TerminalPaneSnapshot};
use serde::Serialize;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Header {
    /// Why the terminal is not live, drawn in the pane's body.
    pub notice: Option<Notice>,
    /// What waits on the operator in this pane, drawn on its second row.
    pub band: Option<Band>,
    /// This pane agent's own current working sentence; never a descendant's line.
    pub progress: Option<String>,
    pub working: bool,
}

/// The terminal's own state while it is not live: the device is away, the
/// terminal could not be restored, the agent sleeps, the stream ended or has
/// not started.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Notice {
    pub kind: String,
    pub tone: &'static str,
    pub reason: Option<String>,
    pub since_unix_ms: Option<u64>,
    pub exit_code: Option<i32>,
}

/// The operator's turn: the pane's own approval, answer, block or stop, or a
/// raised descendant's ask. It stays until it is answered; reading the pane
/// only marks it read.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Band {
    pub kind: String,
    pub reason: Option<String>,
    pub since_unix_ms: Option<u64>,
    /// The operator has had the pane focused since the ask arrived.
    pub read: bool,
    /// The pane the band opens: the raised descendant, or for a draft the
    /// parent holding it.
    pub action: Option<Action>,
    pub more: usize,
    /// A raised descendant's band (PRD D-43): the lead ask, whose verb, what,
    /// who and path the band draws; `more` counts the others.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raised: Option<super::escalation::RaisedAsk>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Action {
    pub pane_id: String,
    pub label: String,
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

/// A pane of a device whose helper link is away: a notice that says the pane
/// is unavailable says what the link waits for, so the pane no longer reads
/// as a failure the operator could do something about, or as one nothing can
/// fix.
pub(crate) fn waiting_on_link(mut header: Header, wait: LinkWait) -> Header {
    if let Some(notice) = header
        .notice
        .as_mut()
        .filter(|notice| notice.kind == "unavailable")
    {
        match wait {
            LinkWait::Returns => {
                notice.kind = "reconnecting".to_owned();
                notice.tone = "muted";
            }
            LinkWait::NeedsConsent => {
                notice.kind = "helper_not_allowed".to_owned();
                notice.tone = "warning";
            }
        }
    }
    header
}

pub(crate) fn of(
    pane: &PaneSnapshot,
    agent: Option<&SidebarAgentSnapshot>,
    transport: Option<&TerminalPaneSnapshot>,
    offline: Option<&str>,
) -> Header {
    let notice = |kind: &str, tone, reason, since_unix_ms, exit_code| {
        Some(Notice {
            kind: kind.to_owned(),
            tone,
            reason,
            since_unix_ms,
            exit_code,
        })
    };
    let notice = if let Some(name) = offline {
        notice("device_offline", "muted", Some(name.to_owned()), None, None)
    } else if pane.failed_terminal_id.is_some() {
        notice("restore_failed", "error", None, None, None)
    } else if let Some(sleep) = &pane.sleep {
        notice(
            &sleep.state,
            if sleep.state == "failed" {
                "error"
            } else {
                "muted"
            },
            sleep.reason.clone(),
            Some(sleep.since_unix_ms),
            None,
        )
    } else if let Some(transport) = transport {
        if transport.scroll_held_elsewhere {
            notice("controlled_elsewhere", "muted", None, None, None)
        } else if transport.closed || transport.transport_state == "ended" {
            let failed = transport.exit_code.is_some_and(|code| code != 0);
            notice(
                if failed { "exit" } else { "terminated" },
                if failed { "error" } else { "muted" },
                None,
                None,
                transport.exit_code,
            )
        } else {
            match transport.transport_state.as_str() {
                "released" => notice("disconnected", "muted", None, None, None),
                "closing" => notice("closing", "muted", None, None, None),
                "unavailable" => notice("unavailable", "error", None, None, None),
                "starting" | "connecting" | "reconnecting" => {
                    notice("starting", "muted", None, None, None)
                }
                _ => None,
            }
        }
    } else {
        None
    };
    // A terminal that is not live takes no answer, so nothing waits in it.
    if notice.is_some() {
        return Header {
            notice,
            band: None,
            progress: None,
            working: false,
        };
    }
    let Some(agent) = agent else {
        return Header::default();
    };
    let own = own_ask(agent);
    // The pane's own approval, answer or block comes before a raised
    // descendant's ask; a stop comes after it.
    if !matches!(own, Some("approval" | "answer" | "blocked"))
        && let Some(lead) = agent.raised.first()
    {
        return Header {
            notice: None,
            progress: None,
            working: false,
            band: Some(Band {
                kind: "raised".to_owned(),
                reason: None,
                since_unix_ms: lead.since_unix_ms,
                read: false,
                action: Some(Action {
                    pane_id: lead.open_pane_id.clone(),
                    label: lead.title.clone(),
                }),
                more: agent.raised.len() - 1,
                raised: Some(lead.clone()),
            }),
        };
    }
    let Some(kind) = own else {
        let working = agent.state.verb == super::RequestVerb::Working;
        return Header {
            notice: None,
            band: None,
            progress: working
                .then_some(agent.progress.as_deref())
                .flatten()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned),
            working,
        };
    };
    Header {
        notice: None,
        progress: None,
        working: false,
        band: Some(Band {
            kind: kind.to_owned(),
            // An Answer says what the row's ask says (`detail`), so a native
            // question reads as itself here too, not as the label's line
            // about the work around it. An Approval has no command the core
            // read, and that line is not one, so it carries no reason.
            reason: match kind {
                "answer" => agent.detail.clone(),
                "approval" => None,
                _ => agent
                    .request
                    .as_ref()
                    .and_then(|request| request.line.clone()),
            },
            since_unix_ms: agent.state.request_since,
            read: !agent.unread,
            action: None,
            more: if kind == "stopped" {
                0
            } else {
                agent.raised.len()
            },
            raised: None,
        }),
    }
}

/// The pane's own turn, from its request verb (docs/status-model.md, Quiet
/// pane headers). A question the operator has read still waits for a reply,
/// as its row's mark does, so reading the pane does not take the band away.
fn own_ask(agent: &SidebarAgentSnapshot) -> Option<&'static str> {
    use super::RequestVerb;
    use super::escalation::Verb;
    match agent.state.verb {
        RequestVerb::Answer => Some(match super::turn::demand_verb(agent) {
            Some(Verb::Approval) => "approval",
            _ => "answer",
        }),
        RequestVerb::Blocked => Some("blocked"),
        RequestVerb::Stopped => Some("stopped"),
        RequestVerb::Working => None,
        _ if agent.demand == "question" => Some("answer"),
        _ => None,
    }
}
