//! Bounded, content-free display calculations, prepared on the ledger worker.
//! The shell selects an already calculated range and formats its numbers.

use std::collections::{BTreeMap, BTreeSet};

use hide_session::{Agent, usage::Tokens};
use jiff::{ToSpan, civil::Date};
use serde::Serialize;

use super::ai_usage::Feature;
use super::store::AiRow;
use super::{DAILY_DAYS, DAILY_LIMIT, local_day, prices, store::Daily, worker::Progress};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Cost {
    pub micro_usd: u64,
    pub partial: bool,
    pub waiting: bool,
    pub stale: bool,
    pub incomplete: bool,
    pub unpriced_models: Vec<String>,
    pub other_unpriced_models: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Amount {
    /// Zero calls means no data, never a known zero-dollar bill.
    pub calls: u64,
    pub new_tokens: u64,
    pub tokens: Tokens,
    pub cost: Cost,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Contribution {
    pub agent: String,
    pub new_tokens: u64,
    pub share_basis_points: u16,
}

/// One feature's known calls on a day. Calls that reported no usage are not
/// here: they are a count on the day, never tokens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HideAiFeature {
    pub feature: String,
    pub amount: Amount,
    /// Provider ids of the models that answered, for the row's marks.
    pub providers: Vec<String>,
}

/// Hide's own background AI on one day, already part of the day's amount.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HideAiDay {
    /// The sum of `features`; a lower bound when `unknown_calls` is not zero.
    pub amount: Amount,
    /// Its share of the day's new tokens, over the known tokens only.
    pub share_basis_points: u16,
    pub features: Vec<HideAiFeature>,
    /// Attempts that were submitted and reported no usage. A count of calls,
    /// not tokens, so it is in no amount.
    pub unknown_calls: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Day {
    pub date: String,
    pub week: u16,
    pub weekday: u8,
    pub today: bool,
    pub recent_week: bool,
    pub level: u8,
    pub pending: bool,
    pub amount: Amount,
    pub top_agents: Vec<Contribution>,
    pub agents: Vec<Contribution>,
    pub other_agents: usize,
    /// Only the all-agents range carries it; an agent's own range is that
    /// agent's sessions.
    pub hide_ai: Option<HideAiDay>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Peak {
    pub date: String,
    pub new_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Range {
    /// Inclusive civil-date bounds. Empty dates are drawn by the shell.
    pub calendar_start: String,
    pub calendar_end: String,
    /// Only dates with recorded calls, including calls with zero new tokens.
    pub days: Vec<Day>,
    pub amount: Amount,
    pub since: Option<String>,
    pub active_days: u16,
    pub observed_days: u16,
    pub peak: Option<Peak>,
    pub current_streak: u16,
    pub longest_streak: u16,
    pub reading: bool,
    pub limited: bool,
    pub files_done: usize,
    pub files_total: usize,
    /// The first retained day Hide AI was counted on; nothing before it is.
    /// Only the all-agents range carries it.
    pub hide_ai_since: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AgentRange {
    pub agent: String,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Share {
    pub contribution: Contribution,
    /// Retained history, independent of the displayed grid or selection.
    pub since: Option<String>,
    pub lifetime: Amount,
    pub providers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Surface {
    pub all: Range,
    pub filtered: Vec<AgentRange>,
    /// Selecting an agent never removes the other agents from this list.
    pub shares: Vec<Share>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Projection {
    pub popover: Surface,
    pub year: Surface,
    pub models: Vec<ModelDay>,
    pub models_limited: bool,
}

/// A recorded model's contribution, never inferred from an agent's total.
/// Hide AI uses per-model reports here, not the headline's backend attempts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelDay {
    pub date: String,
    pub agent: String,
    pub model: Option<String>,
    pub hide_ai: bool,
    pub amount: Amount,
}

/// One shared model list rather than a copy for every agent/calendar.
const MODEL_ROW_LIMIT: usize = 4096;
type ModelKey = (Date, bool, String, String);

struct Models {
    start: Date,
    rows: BTreeMap<ModelKey, Sum>,
    limited: bool,
}

impl Models {
    fn new(start: Date) -> Self {
        Self {
            start,
            rows: BTreeMap::new(),
            limited: false,
        }
    }

    fn add(
        &mut self,
        date: Date,
        agent: &str,
        model: &str,
        hide_ai: bool,
        amount: &Sum,
    ) -> Result<(), &'static str> {
        if date < self.start {
            return Ok(());
        }
        self.rows
            .entry((date, hide_ai, agent.into(), model.into()))
            .or_default()
            .merge(amount)?;
        if self.rows.len() > MODEL_ROW_LIMIT {
            // Stable newest-first coverage; an omitted row is reported, not
            // attributed to a fabricated model or added to a model subtotal.
            self.rows.pop_first();
            self.limited = true;
        }
        Ok(())
    }

    fn view(self) -> Result<Vec<ModelDay>, &'static str> {
        self.rows
            .into_iter()
            .rev()
            .map(|((date, hide_ai, agent, model), sum)| {
                Ok(ModelDay {
                    date: date.to_string(),
                    agent,
                    model: (model != hide_session::usage::UNNAMED_MODEL).then_some(model),
                    hide_ai,
                    amount: sum.amount()?,
                })
            })
            .collect()
    }
}

#[derive(Clone, Default)]
struct Sum {
    tokens: Tokens,
    calls: u64,
    cost: Cost,
    unpriced: BTreeSet<String>,
    providers: BTreeSet<String>,
}

fn plus(a: &mut u64, b: u64) -> Result<(), &'static str> {
    *a = a.checked_add(b).ok_or("usage_total_overflow")?;
    Ok(())
}

impl Sum {
    fn merge(&mut self, other: &Self) -> Result<(), &'static str> {
        plus(&mut self.calls, other.calls)?;
        plus(&mut self.tokens.input, other.tokens.input)?;
        plus(&mut self.tokens.output, other.tokens.output)?;
        plus(&mut self.tokens.cache_read, other.tokens.cache_read)?;
        plus(&mut self.tokens.cache_write, other.tokens.cache_write)?;
        plus(&mut self.tokens.cache_write_1h, other.tokens.cache_write_1h)?;
        if let Some(value) = other.tokens.reasoning {
            plus(self.tokens.reasoning.get_or_insert(0), value)?;
        }
        plus(&mut self.cost.micro_usd, other.cost.micro_usd)?;
        self.cost.partial |= other.cost.partial;
        self.cost.waiting |= other.cost.waiting;
        self.cost.stale |= other.cost.stale;
        self.cost.incomplete |= other.cost.incomplete;
        self.unpriced.extend(other.unpriced.iter().cloned());
        self.providers.extend(other.providers.iter().cloned());
        if self.providers.len() > 32 {
            return Err("usage_provider_capacity");
        }
        Ok(())
    }

    fn amount(&self) -> Result<Amount, &'static str> {
        let mut cost = self.cost.clone();
        cost.unpriced_models = self.unpriced.iter().take(8).cloned().collect();
        cost.other_unpriced_models = self.unpriced.len().saturating_sub(8);
        Ok(Amount {
            calls: self.calls,
            new_tokens: self.new_tokens()?,
            tokens: self.tokens.clone(),
            cost,
        })
    }

    fn new_tokens(&self) -> Result<u64, &'static str> {
        self.tokens
            .input
            .checked_add(self.tokens.output)
            .and_then(|n| n.checked_add(self.tokens.cache_write))
            .ok_or("usage_total_overflow")
    }
}

type Days = BTreeMap<Date, BTreeMap<String, Sum>>;

/// Hide AI's rows of one day grouped by feature.
#[derive(Default)]
struct HideDay {
    features: BTreeMap<Feature, Sum>,
    unknown: u64,
}

impl HideDay {
    fn total(&self) -> Result<Sum, &'static str> {
        let mut total = Sum::default();
        for sum in self.features.values() {
            total.merge(sum)?;
        }
        // Calls without a report make the known amounts a lower bound.
        total.cost.partial |= self.unknown > 0;
        Ok(total)
    }

    fn view(&self, day_new_tokens: u64) -> Result<HideAiDay, &'static str> {
        let amount = self.total()?.amount()?;
        Ok(HideAiDay {
            share_basis_points: contribution("hide_ai", amount.new_tokens, day_new_tokens)
                .share_basis_points,
            amount,
            features: self
                .features
                .iter()
                .map(|(feature, sum)| {
                    Ok(HideAiFeature {
                        feature: feature.id().into(),
                        amount: sum.amount()?,
                        providers: sum.providers.iter().cloned().collect(),
                    })
                })
                .collect::<Result<_, &'static str>>()?,
            unknown_calls: self.unknown,
        })
    }
}

/// Hide AI's days and the facts about the whole retained history.
#[derive(Default)]
struct HideView {
    days: BTreeMap<Date, HideDay>,
    since: Option<Date>,
    limited: bool,
}

/// Hide AI's stored rows as the projection reads them.
pub struct Hide<'a> {
    pub rows: &'a [AiRow],
    pub limited: bool,
}

/// Only the own-node records enter the projection. Dates were assigned at
/// ingestion; changing the current time zone never re-buckets those records.
pub fn prepare(
    rows: &[Daily],
    hide: &Hide<'_>,
    progress: &[Progress],
    table: Option<&prices::Table>,
    device: &str,
    now: u64,
    zone: &jiff::tz::TimeZone,
) -> Result<Option<Projection>, &'static str> {
    if rows.len() > DAILY_LIMIT
        || hide.rows.len() > super::store::AI_ROW_LIMIT
        || progress.len() > Agent::supported().count()
    {
        return Err("usage_projection_capacity");
    }
    let today: Date = local_day(now, zone)?
        .parse()
        .map_err(|_| "usage_day_invalid")?;
    let cutoff = shift(today, -(DAILY_DAYS - 1))?;
    let reading = progress.iter().any(|p| p.reading);
    let mut days = Days::new();
    let mut lifetime = BTreeMap::<String, Sum>::new();
    let mut models = Models::new(if reading {
        sunday(shift(today, -45)?)?
    } else {
        shift(sunday(today)?, -52 * 7)?
    });
    for row in rows.iter().filter(|row| row.device == device) {
        if !Agent::supported().any(|agent| agent != Agent::Cursor && agent.as_str() == row.agent) {
            return Err("usage_agent_invalid");
        }
        let date: Date = row.day.parse().map_err(|_| "usage_day_invalid")?;
        if date < cutoff || date > today {
            continue;
        }
        let resolved = prices::cost(row, table, now)?;
        let mut sum = Sum {
            calls: row.totals.calls,
            tokens: row.totals.tokens.clone(),
            cost: Cost {
                micro_usd: resolved.micro_usd,
                partial: resolved.unpriced || resolved.waiting || resolved.incomplete,
                waiting: resolved.waiting,
                stale: resolved.stale,
                incomplete: resolved.incomplete,
                ..Cost::default()
            },
            providers: row.totals.providers.iter().cloned().collect(),
            ..Sum::default()
        };
        // A call whose records name no model (agy's) is unpriced under no
        // model's name.
        if resolved.unpriced && row.model != hide_session::usage::UNNAMED_MODEL {
            sum.unpriced.insert(row.model.clone());
        }
        models.add(date, &row.agent, &row.model, false, &sum)?;
        days.entry(date)
            .or_default()
            .entry(row.agent.clone())
            .or_default()
            .merge(&sum)?;
        lifetime.entry(row.agent.clone()).or_default().merge(&sum)?;
    }
    let hide = hide_view(hide, table, device, (today, cutoff), now, &mut models)?;
    let first = days
        .keys()
        .chain(hide.days.keys())
        .min()
        .copied()
        .or(reading.then_some(today));
    let Some(first) = first else {
        return Ok(None);
    };
    // One stable calendar during backfill and across agent selections.
    let start = if reading { shift(today, -45)? } else { first };
    let models_limited = models.limited;
    Ok(Some(Projection {
        popover: surface(&days, &hide, &lifetime, progress, today, start, 20)?,
        year: surface(&days, &hide, &lifetime, progress, today, start, 53)?,
        models: models.view()?,
        models_limited,
    }))
}

/// Groups Hide AI's rows by day and feature, priced by the same table as the
/// sessions. A row with no model counts a provider's unknown calls.
fn hide_view(
    hide: &Hide<'_>,
    table: Option<&prices::Table>,
    device: &str,
    bounds: (Date, Date),
    now: u64,
    models: &mut Models,
) -> Result<HideView, &'static str> {
    let (today, cutoff) = bounds;
    let mut view = HideView {
        limited: hide.limited,
        ..HideView::default()
    };
    for row in hide.rows {
        let date: Date = row.day.parse().map_err(|_| "usage_day_invalid")?;
        view.since = Some(view.since.map_or(date, |before| before.min(date)));
        if date < cutoff || date > today {
            continue;
        }
        let feature = Feature::parse(&row.feature).unwrap_or(Feature::Other);
        let day = view.days.entry(date).or_default();
        plus(&mut day.unknown, row.unknown)?;
        if row.model.is_empty() {
            continue;
        }
        let resolved = prices::cost(
            &Daily {
                device: device.into(),
                agent: row.provider.clone(),
                day: row.day.clone(),
                model: row.model.clone(),
                totals: row.totals.clone(),
            },
            table,
            now,
        )?;
        let mut sum = Sum {
            calls: row.attempts,
            tokens: row.totals.tokens.clone(),
            cost: Cost {
                micro_usd: resolved.micro_usd,
                partial: resolved.unpriced || resolved.waiting || resolved.incomplete,
                waiting: resolved.waiting,
                stale: resolved.stale,
                incomplete: resolved.incomplete,
                ..Cost::default()
            },
            providers: BTreeSet::from([row.provider.clone()]),
            ..Sum::default()
        };
        if resolved.unpriced {
            sum.unpriced.insert(row.model.clone());
        }
        models.add(
            date,
            &row.provider,
            &row.model,
            true,
            &Sum {
                calls: row.totals.calls,
                ..sum.clone()
            },
        )?;
        day.features.entry(feature).or_default().merge(&sum)?;
    }
    Ok(view)
}

fn shift(day: Date, delta: i64) -> Result<Date, &'static str> {
    day.checked_add(delta.days())
        .map_err(|_| "usage_day_invalid")
}

fn sunday(day: Date) -> Result<Date, &'static str> {
    shift(day, -i64::from(day.weekday().to_sunday_zero_offset()))
}

fn surface(
    days: &Days,
    hide: &HideView,
    lifetime: &BTreeMap<String, Sum>,
    progress: &[Progress],
    today: Date,
    start: Date,
    weeks: i64,
) -> Result<Surface, &'static str> {
    let start = sunday(start)?.max(shift(sunday(today)?, -(weeks - 1) * 7)?);
    let all = range(days, Some(hide), progress, today, start, None)?;
    let mut filtered = Vec::new();
    for agent in Agent::supported().filter(|agent| *agent != Agent::Cursor) {
        let name = agent.as_str();
        let range = range(days, None, progress, today, start, Some(name))?;
        filtered.push(AgentRange {
            agent: name.into(),
            range,
        });
    }
    // An agent's share is of what the agents used: Hide AI sits beside them
    // in the headline, so it is not in their denominator.
    let mut sessions = 0u64;
    for agent in &filtered {
        sessions = sessions
            .checked_add(agent.range.amount.new_tokens)
            .ok_or("usage_total_overflow")?;
    }
    let mut shares = Vec::new();
    for agent in &filtered {
        let name = agent.agent.as_str();
        if let Some(sum) = lifetime.get(name) {
            shares.push(Share {
                contribution: contribution(name, agent.range.amount.new_tokens, sessions),
                since: days
                    .iter()
                    .find(|(_, values)| values.contains_key(name))
                    .map(|(date, _)| date.to_string()),
                lifetime: sum.amount()?,
                providers: sum.providers.iter().cloned().collect(),
            });
        }
    }
    shares.sort_by(|a, b| {
        b.contribution
            .new_tokens
            .cmp(&a.contribution.new_tokens)
            .then_with(|| a.contribution.agent.cmp(&b.contribution.agent))
    });
    Ok(Surface {
        all,
        filtered,
        shares,
    })
}

fn contribution(agent: &str, tokens: u64, total: u64) -> Contribution {
    Contribution {
        agent: agent.into(),
        new_tokens: tokens,
        share_basis_points: if total == 0 {
            0
        } else {
            (u128::from(tokens) * 10_000 / u128::from(total)) as u16
        },
    }
}

fn range(
    source: &Days,
    hide: Option<&HideView>,
    progress: &[Progress],
    today: Date,
    start: Date,
    selected: Option<&str>,
) -> Result<Range, &'static str> {
    let progress: Vec<_> = progress
        .iter()
        .filter(|p| selected.is_none_or(|a| a == p.agent.as_str()))
        .collect();
    let reading = progress.iter().any(|p| p.reading);
    let mut range = Range {
        calendar_start: start.to_string(),
        calendar_end: today.to_string(),
        days: Vec::new(),
        amount: Amount::default(),
        since: None,
        active_days: 0,
        observed_days: 0,
        peak: None,
        current_streak: 0,
        longest_streak: 0,
        reading,
        limited: progress.iter().any(|p| p.limited) || hide.is_some_and(|hide| hide.limited),
        files_done: progress.iter().map(|p| p.files_done).sum(),
        files_total: progress.iter().map(|p| p.files_total).sum(),
        hide_ai_since: hide
            .and_then(|hide| hide.since)
            .map(|date| date.to_string()),
    };
    let mut total = Sum::default();
    let mut date = start;
    let mut day_index = 0;
    let mut streak = 0;
    let mut closed = Vec::new();
    while date <= today {
        let mut sum = Sum::default();
        let mut contributors = Vec::new();
        if let Some(values) = source.get(&date) {
            for (agent, value) in values
                .iter()
                .filter(|(agent, _)| selected.is_none_or(|a| a == agent.as_str()))
            {
                sum.merge(value)?;
                contributors.push((agent, value.new_tokens()?));
            }
        }
        // The agents' share is over their own tokens; Hide AI is not an agent.
        let session_new_tokens = sum.new_tokens()?;
        let hide_day = hide.and_then(|hide| hide.days.get(&date));
        if let Some(hide_day) = hide_day {
            sum.merge(&hide_day.total()?)?;
        }
        total.merge(&sum)?;
        let amount = sum.amount()?;
        if amount.calls > 0 && range.since.is_none() {
            range.since = Some(date.to_string());
        }
        if range.since.is_some() {
            range.observed_days += 1;
        }
        if amount.new_tokens > 0 {
            range.active_days += 1;
            streak += 1;
            range.longest_streak = range.longest_streak.max(streak);
            if date < today {
                closed.push(amount.new_tokens);
            }
            if range
                .peak
                .as_ref()
                .is_none_or(|p| amount.new_tokens > p.new_tokens)
            {
                range.peak = Some(Peak {
                    date: date.to_string(),
                    new_tokens: amount.new_tokens,
                });
            }
        } else if date < today {
            streak = 0;
        }
        contributors.sort_by(|(a, x), (b, y)| y.cmp(x).then_with(|| a.cmp(b)));
        let count = contributors.len();
        let agents: Vec<_> = contributors
            .into_iter()
            .map(|(agent, n)| contribution(agent, n, session_new_tokens))
            .collect();
        let top_agents = agents.iter().take(3).cloned().collect();
        if amount.calls > 0 || hide_day.is_some_and(|day| day.unknown > 0) {
            let hide_ai = hide_day
                .map(|day| day.view(amount.new_tokens))
                .transpose()?;
            range.days.push(Day {
                date: date.to_string(),
                week: day_index / 7,
                weekday: date.weekday().to_sunday_zero_offset() as u8,
                today: date == today,
                recent_week: date >= shift(today, -6)?,
                level: 0,
                pending: false,
                amount,
                top_agents,
                agents,
                other_agents: count.saturating_sub(3),
                hide_ai,
            });
        }
        date = shift(date, 1)?;
        day_index += 1;
    }
    range.current_streak = streak;
    range.amount = total.amount()?;
    closed.sort_unstable();
    let cuts = if closed.len() >= 7 {
        [1, 2, 3].map(|q| closed[(closed.len() * q).div_ceil(4) - 1])
    } else {
        // Closed days alone set scale. A first active day still paints as
        // strongest instead of masquerading as an empty level-zero day.
        let max = closed.last().copied().unwrap_or(0);
        [1, 2, 3].map(|q| (u128::from(max) * q / 4) as u64)
    };
    for day in &mut range.days {
        let n = day.amount.new_tokens;
        day.level = if n == 0 {
            0
        } else {
            1 + cuts.iter().filter(|cut| n > **cut).count() as u8
        };
    }
    Ok(range)
}

#[cfg(test)]
mod tests;
