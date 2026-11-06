//! Exact native model ids and standard API-rate equivalents. No aliases,
//! provider routing guesses or inferred cache-price multipliers.

use super::{DAY_MS, store::Daily};
use hide_session::usage::Tokens;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SOURCE_LIMIT: usize = hide_node_link::usage::PRICE_BODY_LIMIT;
pub const CACHE_LIMIT: usize = 2 * 1024 * 1024;
const MODEL_LIMIT: usize = 2048;
pub const STALE_MS: u64 = 60 * DAY_MS;
const THRESHOLDS: [u64; 3] = [100_000, 200_000, 272_000];
const FIELDS: [&str; 5] = [
    "input_cost_per_token",
    "output_cost_per_token",
    "cache_read_input_token_cost",
    "cache_creation_input_token_cost",
    "cache_creation_input_token_cost_above_1hr",
];

/// MIT notice travels with the cached portion of the public dataset.
pub const LICENSE: &str = "MIT License\n\nCopyright (c) 2023 Berri AI\n\nPermission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the \"Software\"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:\n\nThe above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.\n\nTHE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.";

pub fn prompt_band(prompt: u64) -> usize {
    THRESHOLDS.partition_point(|threshold| prompt > *threshold)
}

/// Pico-USD per token; multiplication uses u128 and rounds only the final
/// daily amount to micro-USD. Output already includes reasoning.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rates([Option<u64>; 5]);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Model {
    provider: String,
    bands: [Rates; 4],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    pub fetched_at: u64,
    models: BTreeMap<String, Model>,
    license: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    pub attempted_at: Option<u64>,
    pub table: Option<Table>,
}

impl Cache {
    pub fn due(&self, now: u64) -> bool {
        self.attempted_at
            .is_none_or(|last| now.saturating_sub(last) >= DAY_MS)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(table) = &self.table {
            if table.models.is_empty()
                || table.models.len() > MODEL_LIMIT
                || table.license != LICENSE
            {
                return Err("usage_prices_invalid");
            }
            for (id, model) in &table.models {
                if id.is_empty()
                    || id.len() > 128
                    || id.chars().any(char::is_control)
                    || !matches!(model.provider.as_str(), "anthropic" | "openai")
                    || model.bands.iter().any(|band| {
                        band.0
                            .iter()
                            .flatten()
                            .any(|rate| *rate > 1_000_000_000_000)
                    })
                {
                    return Err("usage_prices_invalid");
                }
            }
        }
        Ok(())
    }
}

/// One public, constant URL with no credentials, query, model ids or usage
/// payload. The owned thread has a global deadline and is joined on drop.
pub(crate) struct Fetch(Option<std::thread::JoinHandle<Result<Table, &'static str>>>);

impl Fetch {
    pub(crate) fn start(
        own: std::sync::Arc<dyn hide_node_link::NodeLink>,
    ) -> Result<Self, &'static str> {
        let join = std::thread::Builder::new()
            .name("hide-usage-prices".into())
            .spawn(move || download(own.as_ref()))
            .map_err(|_| "usage_prices_start_failed")?;
        Ok(Self(Some(join)))
    }

    pub(crate) fn take(&mut self) -> Option<Result<Table, &'static str>> {
        if !self.0.as_ref()?.is_finished() {
            return None;
        }
        Some(
            self.0
                .take()?
                .join()
                .unwrap_or(Err("usage_prices_worker_failed")),
        )
    }
}

impl Drop for Fetch {
    fn drop(&mut self) {
        if let Some(join) = self.0.take() {
            let _ = join.join();
        }
    }
}

fn download(own: &dyn hide_node_link::NodeLink) -> Result<Table, &'static str> {
    use hide_node_link::{
        call_as,
        protocol::Call,
        usage::{PriceFailure, PriceFetch},
    };
    let body = call_as::<PriceFetch>(
        own,
        Call::UsagePriceFetch,
        std::time::Duration::from_secs(6),
    )
    .map_err(|_| "usage_prices_node_failed")?
    .map_err(|failure| match failure {
        PriceFailure::Fetch => "usage_prices_fetch_failed",
        PriceFailure::Body => "usage_prices_body_failed",
    })?;
    Table::parse(body.as_bytes(), crate::links::now_ms())
}

impl Table {
    /// Keep only first-party Anthropic and OpenAI text-model rows. Other
    /// providers' native recorded costs never need a price-table estimate.
    pub fn parse(bytes: &[u8], fetched_at: u64) -> Result<Self, &'static str> {
        if bytes.len() > SOURCE_LIMIT {
            return Err("usage_prices_capacity");
        }
        let rows: BTreeMap<String, serde_json::Value> =
            serde_json::from_slice(bytes).map_err(|_| "usage_prices_invalid")?;
        if rows.len() > 16_384 {
            return Err("usage_prices_capacity");
        }
        let mut models = BTreeMap::new();
        for (id, row) in rows {
            let Some(provider @ ("anthropic" | "openai")) =
                row.get("litellm_provider").and_then(|v| v.as_str())
            else {
                continue;
            };
            if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
                continue;
            }
            let Some(object) = row.as_object() else {
                continue;
            };
            // A newly introduced context boundary needs a matching rollup
            // band. Refuse that model instead of applying its base rate.
            let unknown_tier = object.keys().any(|key| {
                FIELDS
                    .iter()
                    .rev()
                    .find_map(|field| key.strip_prefix(field))
                    .is_some_and(|suffix| {
                        suffix.starts_with("_above_")
                            && suffix.ends_with("_tokens")
                            && !THRESHOLDS
                                .iter()
                                .any(|n| suffix == format!("_above_{}k_tokens", n / 1000))
                    })
            });
            if unknown_tier {
                continue;
            }
            let read_rates = |suffix: &str| -> Option<Rates> {
                let mut rates = Rates::default();
                for (index, field) in FIELDS.iter().enumerate() {
                    if let Some(value) = object.get(&format!("{field}{suffix}")) {
                        let value = value.as_f64()?;
                        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                            return None;
                        }
                        let pico = (value * 1_000_000_000_000.0).round();
                        if value > 0.0 && pico == 0.0 {
                            return None;
                        }
                        rates.0[index] = Some(pico as u64);
                    }
                }
                (rates.0[0].is_some() && rates.0[1].is_some()).then_some(rates)
            };
            let Some(base) = read_rates("") else {
                continue;
            };
            let mut bands = [base.clone(), base.clone(), base.clone(), base];
            let mut valid = true;
            for (index, threshold) in THRESHOLDS.iter().enumerate() {
                let suffix = format!("_above_{}k_tokens", threshold / 1000);
                if FIELDS
                    .iter()
                    .any(|field| object.contains_key(&format!("{field}{suffix}")))
                {
                    let Some(tier) = read_rates(&suffix) else {
                        valid = false;
                        break;
                    };
                    for band in &mut bands[index + 1..] {
                        *band = tier.clone();
                    }
                }
            }
            if !valid {
                continue;
            }
            if models.len() == MODEL_LIMIT {
                return Err("usage_prices_capacity");
            }
            models.insert(
                id,
                Model {
                    provider: provider.to_owned(),
                    bands,
                },
            );
        }
        if models.is_empty() {
            return Err("usage_prices_empty");
        }
        Ok(Self {
            fetched_at,
            models,
            license: LICENSE.to_owned(),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cost {
    pub micro_usd: u64,
    pub unpriced: bool,
    pub waiting: bool,
    pub stale: bool,
    pub incomplete: bool,
}

/// Native costs remain authoritative. Estimates apply only to Claude and
/// Codex, and missing nonzero token-category rates make the result partial.
pub fn cost(day: &Daily, table: Option<&Table>, now: u64) -> Result<Cost, &'static str> {
    let mut cost = Cost {
        micro_usd: day.totals.recorded_cost_micro_usd,
        incomplete: day.totals.incomplete_calls > 0,
        ..Cost::default()
    };
    if day.totals.missing_cost_calls == 0 {
        return Ok(cost);
    }
    let provider = match day.agent.as_str() {
        "claude" => "anthropic",
        "codex" => "openai",
        _ => {
            cost.unpriced = day.totals.missing_cost_calls > 0;
            return Ok(cost);
        }
    };
    if day.totals.missing_cost_calls != day.totals.calls {
        cost.unpriced = true;
        return Ok(cost);
    }
    let Some(table) = table else {
        cost.waiting = true;
        return Ok(cost);
    };
    cost.stale = now.saturating_sub(table.fetched_at) > STALE_MS;
    let Some(model) = table
        .models
        .get(&day.model)
        .filter(|model| model.provider == provider)
    else {
        cost.unpriced = true;
        return Ok(cost);
    };
    let mut pico = 0u128;
    let mut accounted = 0u128;
    for (tokens, rates) in day.totals.price_bands.iter().zip(&model.bands) {
        let values = token_values(tokens)?;
        accounted += values.iter().map(|n| u128::from(*n)).sum::<u128>();
        for (count, rate) in values.into_iter().zip(rates.0) {
            if count == 0 {
                continue;
            }
            if let Some(rate) = rate {
                pico += u128::from(count) * u128::from(rate);
            } else {
                cost.unpriced = true;
            }
        }
    }
    // A pre-bands cache can retain its tokens without inventing a pricing
    // split that was never recorded.
    let expected = token_values(&day.totals.tokens)?
        .iter()
        .map(|n| u128::from(*n))
        .sum::<u128>();
    cost.unpriced |= accounted != expected;
    cost.micro_usd = cost
        .micro_usd
        .checked_add(
            u64::try_from((pico + 500_000) / 1_000_000).map_err(|_| "usage_cost_overflow")?,
        )
        .ok_or("usage_cost_overflow")?;
    Ok(cost)
}

fn token_values(tokens: &Tokens) -> Result<[u64; 5], &'static str> {
    Ok([
        tokens.input,
        tokens.output,
        tokens.cache_read,
        tokens
            .cache_write
            .checked_sub(tokens.cache_write_1h)
            .ok_or("usage_tokens_invalid")?,
        tokens.cache_write_1h,
    ])
}

#[cfg(test)]
mod tests;
