//! A project's last seven days of cost (PRD lane-two-columns D-19, B31),
//! prepared on the ledger worker from the 45-day `events` table: the calls
//! whose session ran inside one of the project's checkouts, priced the way a
//! day is (`prices::cost`). The daily totals and the Overview's Usage are
//! untouched; a call outside every checkout counts for no project.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use hide_session::usage::Event;
use jiff::{ToSpan, civil::Date};
use serde::Serialize;

use super::prices::{self, Table};
use super::store::{Daily, Totals};

/// The days a project's cost covers, today included.
pub const DAYS: i64 = 7;
/// The longest quiet between two calls of a session still counted as its
/// working time (PRD project-panel-results D-11): a longer gap is the agent
/// waiting, not working.
pub const WORK_GAP_MS: u64 = 5 * 60_000;
/// The model rows one project publishes.
pub const MODEL_LIMIT: usize = 16;
/// The calls one preparation reads; past it the newest are read and the
/// cost says it is partial (engineering principle 15).
pub const EVENT_LIMIT: usize = 200_000;

/// The checkouts of one project on the core's own machine, as the catalog
/// names them; a call is the project's when its session's folder is inside
/// one, the deepest root deciding between two projects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectRoots {
    pub workspace_id: String,
    pub roots: Vec<String>,
}

/// The projects' costs as a usage frame carries them (B31, B33): `built` is
/// absent until a build succeeded, so a tile tells a cost not read yet from
/// a read that found none; `failed` says the latest build failed, the last
/// built list kept.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProjectCosts {
    pub built: Option<std::sync::Arc<Vec<ProjectCost>>>,
    pub failed: bool,
    /// What Hide AI's own calls cost over the same seven days, in no
    /// project and no task (PRD project-panel-results D-10, B24); absent
    /// until a build succeeded.
    pub hide_ai_micro_usd: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ProjectCost {
    pub workspace_id: String,
    /// The sum of the priced amounts, in millionths of a dollar.
    pub micro_usd: u64,
    /// New tokens of the seven days' calls: input, output and cache writes.
    pub tokens: u64,
    /// Oldest first, one per day of the seven, today last.
    pub days: Vec<DayCost>,
    pub models: Vec<ModelCost>,
    /// Each priced agent's amount, largest first.
    pub agents: Vec<AgentCost>,
    /// Agents that ran a call here Hide has no price for (B31): named apart,
    /// those calls in no amount, the agent's priced ones still counted.
    pub unpriced_agents: Vec<String>,
    /// Some calls had a price missing, also from an agent whose other calls
    /// are priced: the amount is a lower bound.
    pub partial: bool,
    /// The public price table has not been read yet.
    pub waiting: bool,
    pub stale: bool,
    /// More calls than `EVENT_LIMIT` fell in the seven days.
    pub limited: bool,
    /// What each session spent here, which the core groups into tasks with
    /// what it knows of the sessions (`usage_ledger::tasks`). Core only.
    #[serde(skip)]
    pub spends: Vec<SessionSpend>,
    /// The seven days' tasks, costliest first, at most `tasks::TASK_LIMIT`
    /// (PRD project-panel-results B27-B34); the core fills them.
    pub tasks: Vec<super::tasks::TaskCost>,
    pub tasks_total: usize,
    pub week: super::tasks::ProjectWeek,
}

/// One session's seven days in a project (PRD project-panel-results D-09,
/// D-11): its priced amount, whether some of it had no price, its new
/// tokens, its first and last call and its working time.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionSpend {
    pub agent: String,
    pub session_id: String,
    pub micro_usd: u64,
    pub unpriced: bool,
    pub tokens: u64,
    pub first_ms: u64,
    pub last_ms: u64,
    /// The gaps between its calls up to `WORK_GAP_MS`, added up.
    pub worked_ms: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct DayCost {
    pub date: String,
    pub micro_usd: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModelCost {
    pub agent: String,
    /// Absent for a model the native records do not name.
    pub model: Option<String>,
    pub micro_usd: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentCost {
    pub agent: String,
    pub micro_usd: u64,
}

/// The first day of the seven that end on `today`.
pub fn first_day(today: Date) -> Result<Date, &'static str> {
    today
        .checked_sub((DAYS - 1).days())
        .map_err(|_| "usage_date_invalid")
}

/// The oldest call time a preparation at `now` reads. A day past the seven
/// reaches back eight, so a time zone change cannot drop the first day's
/// earliest calls; a call older than this is in no project's cost.
pub fn since(now: u64) -> u64 {
    now.saturating_sub((DAYS as u64 + 1) * super::DAY_MS)
}

#[derive(Default)]
struct Groups {
    /// (agent, day, model) totals, priced as a day is.
    days: BTreeMap<(String, String, String), Totals>,
    /// (agent, session, model) totals.
    sessions: BTreeMap<(String, String, String), Totals>,
    /// (agent, session): new tokens and every call's time, for the
    /// session's span and working time.
    calls: BTreeMap<(String, String), (u64, Vec<u64>)>,
}

/// Every project's cost over the seven days ending `today`, prepared from
/// calls as the store reads them: each call goes into its project's group
/// totals and is dropped, so what is held grows with the (agent, day,
/// model) and (agent, session, model) groups, not with the calls.
pub struct Preparation<'a> {
    projects: &'a [ProjectRoots],
    first: Date,
    first_text: String,
    groups: BTreeMap<&'a str, Groups>,
    /// Each session folder's project, decided once: a session's calls
    /// share its folder, so this grows with the sessions as `groups` does.
    owners: HashMap<String, Option<&'a str>>,
}

impl<'a> Preparation<'a> {
    pub fn new(projects: &'a [ProjectRoots], today: Date) -> Result<Self, &'static str> {
        let first = first_day(today)?;
        Ok(Self {
            projects,
            first,
            first_text: first.to_string(),
            groups: BTreeMap::new(),
            owners: HashMap::new(),
        })
    }

    /// One call of `agent` the store assigned to `day`.
    pub fn add(&mut self, agent: &str, day: &str, event: &Event) -> Result<(), &'static str> {
        if day < self.first_text.as_str() {
            return Ok(());
        }
        let Some(cwd) = event.cwd.as_deref() else {
            return Ok(());
        };
        let project = match self.owners.get(cwd) {
            Some(project) => *project,
            None => {
                let project = owner(self.projects, cwd);
                self.owners.insert(cwd.to_owned(), project);
                project
            }
        };
        let Some(project) = project else {
            return Ok(());
        };
        let into = self.groups.entry(project).or_default();
        let tokens = &event.tokens;
        let (sum, stamps) = into
            .calls
            .entry((agent.to_owned(), event.session.clone()))
            .or_default();
        for part in [
            tokens.input,
            tokens.output,
            tokens.cache_write,
            tokens.cache_write_1h,
        ] {
            add(sum, part)?;
        }
        stamps.push(event.at_ms);
        for totals in [
            into.days
                .entry((agent.to_owned(), day.to_owned(), event.model.clone()))
                .or_default(),
            into.sessions
                .entry((agent.to_owned(), event.session.clone(), event.model.clone()))
                .or_default(),
        ] {
            totals.add_call(
                &event.tokens,
                event.recorded_cost_micro_usd,
                event.incomplete,
                event.provider.as_deref(),
            )?;
        }
        Ok(())
    }

    /// The projects' costs from the calls added. `limited` says the store
    /// held more calls than were read.
    pub fn finish(
        self,
        limited: bool,
        table: Option<&Table>,
        now: u64,
    ) -> Result<Vec<ProjectCost>, &'static str> {
        let Self {
            projects,
            first,
            first_text,
            groups,
            ..
        } = self;
        projects
            .iter()
            .filter_map(|project| {
                groups
                    .get(project.workspace_id.as_str())
                    .map(|groups| (project, groups))
            })
            .map(|(project, groups)| {
                let mut cost = ProjectCost {
                    workspace_id: project.workspace_id.clone(),
                    limited,
                    ..ProjectCost::default()
                };
                let mut days: BTreeMap<String, u64> = (0..DAYS)
                    .map(|offset| {
                        first
                            .checked_add(offset.days())
                            .map(|date| (date.to_string(), 0))
                            .map_err(|_| "usage_date_invalid")
                    })
                    .collect::<Result<_, _>>()?;
                let mut models: BTreeMap<(String, String), u64> = BTreeMap::new();
                let mut agents: BTreeMap<String, u64> = BTreeMap::new();
                let mut unpriced: BTreeSet<String> = BTreeSet::new();
                for ((agent, day, model), totals) in &groups.days {
                    let priced = price(agent, day, model, totals, table, now)?;
                    cost.partial |= priced.unpriced && priced.micro_usd > 0;
                    cost.waiting |= priced.waiting;
                    cost.stale |= priced.stale;
                    if priced.unpriced {
                        unpriced.insert(agent.clone());
                    }
                    if priced.micro_usd == 0 {
                        continue;
                    }
                    add(&mut cost.micro_usd, priced.micro_usd)?;
                    add(days.entry(day.clone()).or_default(), priced.micro_usd)?;
                    add(
                        models.entry((agent.clone(), model.clone())).or_default(),
                        priced.micro_usd,
                    )?;
                    add(agents.entry(agent.clone()).or_default(), priced.micro_usd)?;
                }
                let mut sessions: BTreeMap<(String, String), (u64, bool)> = BTreeMap::new();
                for ((agent, session, model), totals) in &groups.sessions {
                    let priced = price(agent, &first_text, model, totals, table, now)?;
                    let (micro_usd, unpriced) = sessions
                        .entry((agent.clone(), session.clone()))
                        .or_default();
                    add(micro_usd, priced.micro_usd)?;
                    *unpriced |= priced.unpriced;
                }
                for ((agent, session_id), (tokens, stamps)) in &groups.calls {
                    let (micro_usd, unpriced) = sessions
                        .get(&(agent.clone(), session_id.clone()))
                        .copied()
                        .unwrap_or_default();
                    add(&mut cost.tokens, *tokens)?;
                    cost.spends.push(spend(
                        agent, session_id, micro_usd, unpriced, *tokens, stamps,
                    ));
                }
                cost.days = days
                    .into_iter()
                    .map(|(date, micro_usd)| DayCost { date, micro_usd })
                    .collect();
                cost.models = largest(models, MODEL_LIMIT, |(agent, model), micro_usd| ModelCost {
                    agent,
                    model: (model != hide_session::usage::UNNAMED_MODEL).then_some(model),
                    micro_usd,
                });
                let agents_priced: BTreeSet<String> = agents.keys().cloned().collect();
                cost.agents = largest(agents, usize::MAX, |agent, micro_usd| AgentCost {
                    agent,
                    micro_usd,
                });
                // An agent with a priced amount and an unpriced call leaves the
                // total short, as a partly priced call does.
                cost.partial |= unpriced.iter().any(|agent| agents_priced.contains(agent));
                cost.unpriced_agents = unpriced.into_iter().collect();
                Ok(cost)
            })
            .collect()
    }
}

/// A session's spend from its calls' times: the first and last call, and
/// every gap up to `WORK_GAP_MS` added up as its working time.
fn spend(
    agent: &str,
    session_id: &str,
    micro_usd: u64,
    unpriced: bool,
    tokens: u64,
    stamps: &[u64],
) -> SessionSpend {
    let mut sorted = stamps.to_vec();
    sorted.sort_unstable();
    let worked_ms = sorted
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|gap| *gap <= WORK_GAP_MS)
        .sum();
    SessionSpend {
        agent: agent.to_owned(),
        session_id: session_id.to_owned(),
        micro_usd,
        unpriced,
        tokens,
        first_ms: sorted.first().copied().unwrap_or_default(),
        last_ms: sorted.last().copied().unwrap_or_default(),
        worked_ms,
    }
}

/// What Hide AI's own calls cost from `first` on, priced as a day is.
pub fn hide_ai_cost(
    rows: &[super::store::AiRow],
    first: Date,
    table: Option<&Table>,
    now: u64,
) -> Result<u64, &'static str> {
    let first = first.to_string();
    let mut sum = 0;
    for row in rows
        .iter()
        .filter(|row| row.day >= first && !row.model.is_empty())
    {
        add(
            &mut sum,
            price(&row.provider, &row.day, &row.model, &row.totals, table, now)?.micro_usd,
        )?;
    }
    Ok(sum)
}

/// The project whose deepest checkout holds `cwd`.
fn owner<'a>(projects: &'a [ProjectRoots], cwd: &str) -> Option<&'a str> {
    projects
        .iter()
        .flat_map(|project| project.roots.iter().map(move |root| (project, root)))
        .filter(|(_, root)| crate::files::path_inside_root(root, cwd, true).is_ok())
        .max_by_key(|(_, root)| root.len())
        .map(|(project, _)| project.workspace_id.as_str())
}

fn price(
    agent: &str,
    day: &str,
    model: &str,
    totals: &Totals,
    table: Option<&Table>,
    now: u64,
) -> Result<prices::Cost, &'static str> {
    prices::cost(
        &Daily {
            device: String::new(),
            agent: agent.to_owned(),
            day: day.to_owned(),
            model: model.to_owned(),
            totals: totals.clone(),
        },
        table,
        now,
    )
}

fn add(sum: &mut u64, value: u64) -> Result<(), &'static str> {
    *sum = sum.checked_add(value).ok_or("usage_cost_overflow")?;
    Ok(())
}

/// The `limit` largest nonzero amounts, largest first, ties by key.
fn largest<K: Ord + Clone, T>(
    amounts: BTreeMap<K, u64>,
    limit: usize,
    make: impl Fn(K, u64) -> T,
) -> Vec<T> {
    let mut sorted: Vec<_> = amounts.into_iter().filter(|(_, v)| *v > 0).collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    sorted
        .into_iter()
        .take(limit)
        .map(|(key, value)| make(key, value))
        .collect()
}

#[cfg(test)]
mod tests;
