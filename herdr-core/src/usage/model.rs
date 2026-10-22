//! Shared typed usage projection. Reading decisions belong to the core;
//! wording and formatting belong to the shell.
pub use super::consumption::{Consumption, History};
use hide_agent_adapter::usage::Support;
pub use hide_agent_adapter::usage::WindowKind;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UsageSnapshot {
    pub nodes: Vec<NodeUsage>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NodeUsage {
    pub node: String,
    pub agents: Vec<AgentUsage>,
    pub history: Option<History>,
    /// Each of the node's projects' last seven days (PRD lane-two-columns
    /// B31); left off the wire while there are none.
    #[serde(
        skip_serializing_if = "projects_empty",
        serialize_with = "shared_projects"
    )]
    pub projects: std::sync::Arc<Vec<crate::usage_ledger::project_cost::ProjectCost>>,
}

fn projects_empty(
    value: &std::sync::Arc<Vec<crate::usage_ledger::project_cost::ProjectCost>>,
) -> bool {
    value.is_empty()
}

fn shared_projects<S: serde::Serializer>(
    value: &std::sync::Arc<Vec<crate::usage_ledger::project_cost::ProjectCost>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value.as_slice().serialize(serializer)
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AgentUsage {
    pub agent: String,
    pub limits: Cell<LimitReading>,
    pub consumption: Cell<Consumption>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Cell<T> {
    Unsupported,
    Loading,
    Unavailable {
        cause: Cause,
        last_checked_at_ms: Option<u64>,
    },
    Reading {
        value: T,
        freshness: Freshness,
        observed_at_ms: u64,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Fresh,
    Stale,
    Fallback,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    SignedOut,
    NotInstalled,
    Offline,
    TimedOut,
    RateLimited,
    FormatChanged,
    Capped,
    NoRecords,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LimitReading {
    pub windows: Vec<WindowReading>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WindowReading {
    pub kind: WindowKind,
    pub scope: Scope,
    pub outcome: WindowOutcome,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    All,
    Model { label: String },
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum WindowOutcome {
    NotStarted,
    Open { used_percent: f64, resets_at_s: u64 },
    Expired { reset_at_s: u64 },
    NotOffered,
    Unreadable { cause: Cause },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WeeklyUsage {
    Used(f64),
    Unread,
    Unknown,
}

impl AgentUsage {
    pub fn initial_rows() -> Vec<Self> {
        hide_agent_adapter::ADAPTERS
            .iter()
            .map(|row| Self {
                agent: row.herdr.name.into(),
                limits: if row.usage.limit_sources().next().is_some() {
                    Cell::Loading
                } else {
                    Cell::Unsupported
                },
                consumption: match row.usage.tokens {
                    Support::Unsupported(_) => Cell::Unsupported,
                    Support::Supported(_) => Cell::Loading,
                },
            })
            .collect()
    }
}
impl UsageSnapshot {
    pub fn initial(node: &str) -> Self {
        Self {
            nodes: vec![NodeUsage {
                node: node.into(),
                agents: AgentUsage::initial_rows(),
                history: None,
                projects: Default::default(),
            }],
        }
    }
    /// Limits always refer to the core's own node, even while a device is
    /// visible. Later consumption nodes cannot change that ownership.
    pub fn own_agent(&self, agent: &str) -> Option<&AgentUsage> {
        self.nodes
            .first()?
            .agents
            .iter()
            .find(|row| row.agent == agent)
    }
    pub fn weekly(&self, agent: &str, now_s: u64) -> WeeklyUsage {
        if matches!(
            self.own_agent(agent).map(|row| &row.limits),
            Some(Cell::Loading)
        ) {
            return WeeklyUsage::Unread;
        }
        self.weekly_reading(agent, now_s)
            .map_or(WeeklyUsage::Unknown, |(used, _)| WeeklyUsage::Used(used))
    }
    pub fn weekly_reading(&self, agent: &str, now_s: u64) -> Option<(f64, u64)> {
        let Cell::Reading { value, .. } = &self.own_agent(agent)?.limits else {
            return None;
        };
        value.windows.iter().find_map(|window| {
            if window.kind != WindowKind::Weekly || window.scope != Scope::All {
                return None;
            }
            match window.outcome {
                WindowOutcome::Open {
                    used_percent,
                    resets_at_s,
                } if resets_at_s > now_s => Some((used_percent, resets_at_s)),
                _ => None,
            }
        })
    }
    /// Wait through the latest exhausted window, including the five-hour
    /// window and scoped weekly limits. Stale/fallback readings retain the
    /// same reset boundary; an unavailable reading never blocks work.
    pub fn exhausted_until(&self, agent: &str, now_s: u64) -> Option<u64> {
        let Cell::Reading { value, .. } = &self.own_agent(agent)?.limits else {
            return None;
        };
        value
            .windows
            .iter()
            .filter_map(|window| match window.outcome {
                WindowOutcome::Open {
                    used_percent,
                    resets_at_s,
                } if used_percent >= 100.0 && resets_at_s > now_s => Some(resets_at_s),
                _ => None,
            })
            .max()
    }
}
impl LimitReading {
    pub fn expire(&mut self, now_s: u64) {
        for window in &mut self.windows {
            if let WindowOutcome::Open { resets_at_s, .. } = window.outcome
                && resets_at_s <= now_s
            {
                window.outcome = WindowOutcome::Expired {
                    reset_at_s: resets_at_s,
                };
            }
        }
    }
}
