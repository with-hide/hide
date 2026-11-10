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
    /// B31): absent until the first build, so a cost not read yet is not
    /// drawn as zero (B33); a project that ran no priced call is not listed.
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "shared_projects"
    )]
    pub projects: Option<std::sync::Arc<Vec<crate::usage_ledger::project_cost::ProjectCost>>>,
    /// The latest build of `projects` failed; the last one stays.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub projects_failed: bool,
    /// What Hide AI's own calls cost over the same seven days, in no
    /// project (PRD project-panel-results B24); absent until a build.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hide_ai_week_micro_usd: Option<u64>,
}

fn shared_projects<S: serde::Serializer>(
    value: &Option<std::sync::Arc<Vec<crate::usage_ledger::project_cost::ProjectCost>>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value.as_deref().map(Vec::as_slice).serialize(serializer)
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AgentUsage {
    pub agent: String,
    pub limits: Cell<LimitReading>,
    pub consumption: Cell<Consumption>,
    /// The limits of each of this machine's profiles of the agent, in the
    /// list's order (PRD agent-profiles B23); absent with none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<ProfileUsage>,
}

/// One profile's limits: its account's, read for the first profile in it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProfileUsage {
    /// The profile's id.
    pub id: String,
    /// Another profile listed before it, or the agent's default account,
    /// runs in the same account: Settings › Agents draws `—` for it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub shared: bool,
    /// Its account's limits, the same on every profile of that account; absent
    /// for a profile in the agent's default account, whose figure is the
    /// agent's own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<Cell<LimitReading>>,
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
                agent: row.engine_kind.into(),
                limits: if row.usage.limit_sources().next().is_some() {
                    Cell::Loading
                } else {
                    Cell::Unsupported
                },
                consumption: match row.usage.tokens {
                    Support::Unsupported(_) => Cell::Unsupported,
                    Support::Supported(_) => Cell::Loading,
                },
                profiles: Vec::new(),
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
                projects: None,
                projects_failed: false,
                hide_ai_week_micro_usd: None,
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
        Self::weekly_of(&self.own_agent(agent)?.limits, now_s)
    }

    /// [`Self::weekly_reading`] of the account of the profile `id` of
    /// `agent`, as its own row last read it (PRD agent-profiles B23); `None`
    /// for a profile whose row is another's (`shared`) or has no figure.
    pub fn profile_weekly_reading(&self, agent: &str, id: &str, now_s: u64) -> Option<(f64, u64)> {
        let row = self
            .own_agent(agent)?
            .profiles
            .iter()
            .find(|row| row.id == id)?;
        Self::weekly_of(row.limits.as_ref()?, now_s)
    }

    /// The open seven-day window of all models in `limits`, as its share
    /// used and when it resets.
    fn weekly_of(limits: &Cell<LimitReading>, now_s: u64) -> Option<(f64, u64)> {
        let Cell::Reading { value, .. } = limits else {
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
