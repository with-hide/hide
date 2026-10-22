//! The worker prepares owned display values before it takes the runtime lock.
use std::sync::Arc;

use serde::{Serialize, Serializer};

use super::model::{Cause, Cell, Freshness};
use crate::usage_ledger::{query, worker::Report};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Consumption {
    #[serde(serialize_with = "shared")]
    pub popover: Arc<query::Range>,
    #[serde(serialize_with = "shared")]
    pub year: Arc<query::Range>,
    pub retained: Option<Retained>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Retained {
    pub since: Option<String>,
    pub amount: query::Amount,
    pub providers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Surface {
    #[serde(serialize_with = "shared")]
    pub range: Arc<query::Range>,
    pub shares: Vec<query::Share>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct History {
    pub popover: Surface,
    pub year: Surface,
    pub models: Vec<query::ModelDay>,
    pub models_limited: bool,
}

fn shared<S: Serializer, T: Serialize>(value: &Arc<T>, serializer: S) -> Result<S::Ok, S::Error> {
    value.as_ref().serialize(serializer)
}

pub(crate) struct Publication {
    pub history: Option<History>,
    pub agents: Vec<(String, Cell<Consumption>)>,
    pub projects: Arc<Vec<crate::usage_ledger::project_cost::ProjectCost>>,
}

impl Publication {
    pub fn prepare(report: Report, now: u64) -> Self {
        let history = report.projection.as_ref().map(|value| History {
            popover: Surface {
                range: Arc::new(value.popover.all.clone()),
                shares: value.popover.shares.clone(),
            },
            year: Surface {
                range: Arc::new(value.year.all.clone()),
                shares: value.year.shares.clone(),
            },
            models: value.models.clone(),
            models_limited: value.models_limited,
        });
        let agents = hide_agent_adapter::ADAPTERS
            .iter()
            .map(|adapter| {
                let name = adapter.herdr.name;
                let cell = match adapter.usage.tokens {
                    hide_agent_adapter::usage::Support::Unsupported(_) => Cell::Unsupported,
                    hide_agent_adapter::usage::Support::Supported(_) => {
                        let progress = report.progress.iter().find(|p| p.agent.as_str() == name);
                        let ranges = report.projection.as_ref().and_then(|value| {
                            Some((
                                value.popover.filtered.iter().find(|p| p.agent == name)?,
                                value.year.filtered.iter().find(|p| p.agent == name)?,
                                value,
                            ))
                        });
                        match ranges {
                            Some((popover, year, projection)) => Cell::Reading {
                                value: Consumption {
                                    popover: Arc::new(popover.range.clone()),
                                    year: Arc::new(year.range.clone()),
                                    retained: projection
                                        .year
                                        .shares
                                        .iter()
                                        .find(|p| p.contribution.agent == name)
                                        .map(|p| Retained {
                                            since: p.since.clone(),
                                            amount: p.lifetime.clone(),
                                            providers: p.providers.clone(),
                                        }),
                                },
                                freshness: if report.failure.is_some()
                                    || progress.is_some_and(|p| p.failure.is_some())
                                {
                                    Freshness::Stale
                                } else {
                                    Freshness::Fresh
                                },
                                observed_at_ms: now,
                            },
                            None => Cell::Unavailable {
                                cause: if report.failure.is_some()
                                    || progress.is_some_and(|p| p.failure.is_some())
                                {
                                    Cause::FormatChanged
                                } else {
                                    Cause::NoRecords
                                },
                                last_checked_at_ms: Some(now),
                            },
                        }
                    }
                };
                (name.into(), cell)
            })
            .collect();
        Self {
            history,
            agents,
            projects: report.projects,
        }
    }
}
