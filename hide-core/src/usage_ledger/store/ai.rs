//! Hide's own background AI in the same file as the session ledger. The
//! ledger worker is the only writer; a batch is one transaction. A row is a
//! day, provider, model and feature, so it holds no request, subject, prompt
//! or path (PRD hide-ai-usage-split D-09, D-10).
use super::*;

/// One model's tokens from one attempt, already checked and dated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiPart {
    /// First in a contiguous attempt's model parts. Its call is credited to
    /// the first part actually stored, even if the row cap refuses this one.
    pub starts_attempt: bool,
    pub day: String,
    pub provider: String,
    pub feature: String,
    pub model: String,
    pub tokens: Tokens,
    /// Priced as a lower bound: the one-hour cache split was not reported.
    pub lower_bound: bool,
}

/// Attempts that were submitted and reported no usage, on one day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiUnknown {
    pub day: String,
    pub provider: String,
    pub feature: String,
    pub count: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AiRecorded {
    /// Parts and unknown counts whose row did not exist and could not be
    /// made at the row cap. Their loss is kept as `limited`.
    pub refused: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiRow {
    pub day: String,
    pub provider: String,
    /// Empty on the row that counts a provider's unknown calls.
    pub model: String,
    pub feature: String,
    pub totals: Totals,
    /// Backend attempts, independent of the per-model calls used for pricing.
    pub attempts: u64,
    pub unknown: u64,
}

/// Extend only the Hide AI JSON. The session ledger's per-model pricing
/// totals stay unchanged. Experimental rows without this field keep their
/// old count; their original multi-model attempt boundaries were not saved.
#[derive(Default, Serialize, Deserialize)]
struct AiTotals {
    #[serde(flatten)]
    totals: Totals,
    #[serde(default)]
    attempts: Option<u64>,
}

impl AiTotals {
    fn attempts(&self) -> u64 {
        self.attempts.unwrap_or(self.totals.calls)
    }
}

/// Changes the row of `key` (day, provider, model, feature), making it when
/// there is none and the cap allows. `false` means the cap refused it.
fn upsert(
    tx: &rusqlite::Transaction<'_>,
    key: [&str; 4],
    rows: &mut usize,
    change: impl FnOnce(&mut AiTotals, &mut u64) -> Result<(), &'static str>,
) -> Result<bool, &'static str> {
    let [day, provider, model, feature] = key;
    let found: Option<(String, u64)> = tx
        .query_row(
            "SELECT totals,unknown FROM ai_daily WHERE day=?1 AND provider=?2 AND model=?3 AND feature=?4",
            params![day, provider, model, feature],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(failed)?;
    let (mut totals, mut unknown) = match found {
        Some((text, unknown)) => (decode::<AiTotals>(&text)?, unknown),
        None if *rows >= AI_ROW_LIMIT => return Ok(false),
        None => {
            *rows += 1;
            (AiTotals::default(), 0)
        }
    };
    change(&mut totals, &mut unknown)?;
    tx.execute(
        "INSERT INTO ai_daily VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(day,provider,model,feature) DO UPDATE SET totals=excluded.totals,unknown=excluded.unknown",
        params![day, provider, model, feature, encode(&totals)?, unknown],
    )
    .map_err(failed)?;
    Ok(true)
}

fn note_earliest(earliest: &mut Option<String>, day: &str) {
    if earliest.as_deref().is_none_or(|before| day < before) {
        *earliest = Some(day.to_owned());
    }
}

impl Store {
    /// A part or an unknown count lands on its row; a combination with no row
    /// yet is created only below the row cap. A refusal is not an error: the
    /// rest of the batch is kept and the earliest refused day is persisted.
    pub fn ai_record(
        &mut self,
        parts: &[AiPart],
        unknown: &[AiUnknown],
    ) -> Result<AiRecorded, &'static str> {
        for part in parts {
            valid_name(&part.day, 16)?;
            valid_name(&part.provider, 128)?;
            valid_name(&part.feature, 64)?;
            valid_name(&part.model, 128)?;
        }
        for item in unknown {
            valid_name(&item.day, 16)?;
            valid_name(&item.provider, 128)?;
            valid_name(&item.feature, 64)?;
        }
        self.budget();
        let tx = self.connection.transaction().map_err(failed)?;
        let mut rows: usize = tx
            .query_row("SELECT count(*) FROM ai_daily", [], |r| r.get(0))
            .map_err(failed)?;
        let mut outcome = AiRecorded::default();
        let mut earliest_refused: Option<String> = None;
        let mut attempt_counted = false;
        for part in parts {
            if part.starts_attempt {
                attempt_counted = false;
            }
            let key = [
                part.day.as_str(),
                &part.provider,
                &part.model,
                &part.feature,
            ];
            let written = upsert(&tx, key, &mut rows, |totals, _| {
                let mut attempts = totals.attempts();
                plus(&mut attempts, u64::from(!attempt_counted))?;
                totals.attempts = Some(attempts);
                totals
                    .totals
                    .add_call(&part.tokens, None, part.lower_bound, None)
            })?;
            if written {
                attempt_counted = true;
            } else {
                outcome.refused += 1;
                note_earliest(&mut earliest_refused, &part.day);
            }
        }
        for item in unknown {
            let key = [item.day.as_str(), &item.provider, "", &item.feature];
            let written = upsert(&tx, key, &mut rows, |_, count| plus(count, item.count))?;
            if !written {
                outcome.refused += 1;
                note_earliest(&mut earliest_refused, &item.day);
            }
        }
        if let Some(day) = earliest_refused.as_deref() {
            let before: Option<Option<String>> = tx
                .query_row("SELECT limited_day FROM ai_state WHERE slot=1", [], |r| {
                    r.get(0)
                })
                .optional()
                .map_err(failed)?;
            let day = match before.flatten() {
                Some(before) if before.as_str() <= day => before,
                _ => day.to_owned(),
            };
            tx.execute(
                "INSERT INTO ai_state VALUES(1,?1) ON CONFLICT(slot) DO UPDATE SET limited_day=excluded.limited_day",
                [day],
            )
            .map_err(failed)?;
        }
        tx.commit().map_err(failed)?;
        Ok(outcome)
    }

    pub fn ai_rows(&self) -> Result<Vec<AiRow>, &'static str> {
        self.budget();
        let mut query = self
            .connection
            .prepare(
                "SELECT day,provider,model,feature,totals,unknown FROM ai_daily ORDER BY day,provider,model,feature LIMIT ?1",
            )
            .map_err(failed)?;
        let mut rows = query.query([AI_ROW_LIMIT + 1]).map_err(failed)?;
        // The row cap bounds the read; the time budget bounds one stalled
        // step, so it is re-armed every thousand rows rather than spent by a
        // full table, and decoding the totals comes after it.
        let mut raw = Vec::new();
        while let Some(row) = rows.next().map_err(failed)? {
            if raw.len() % 1024 == 0 {
                self.budget();
            }
            raw.push((
                row.get::<_, String>(0).map_err(failed)?,
                row.get::<_, String>(1).map_err(failed)?,
                row.get::<_, String>(2).map_err(failed)?,
                row.get::<_, String>(3).map_err(failed)?,
                row.get::<_, String>(4).map_err(failed)?,
                row.get::<_, u64>(5).map_err(failed)?,
            ));
            if raw.len() > AI_ROW_LIMIT {
                return Err("usage_ai_capacity");
            }
        }
        raw.into_iter()
            .map(|(day, provider, model, feature, totals, unknown)| {
                let totals: AiTotals = decode(&totals)?;
                let attempts = totals.attempts();
                Ok(AiRow {
                    day,
                    provider,
                    model,
                    feature,
                    totals: totals.totals,
                    attempts,
                    unknown,
                })
            })
            .collect()
    }

    /// Whether a call was lost to the row cap on a day still retained.
    pub fn ai_limited(&self) -> Result<bool, &'static str> {
        self.budget();
        self.connection
            .query_row(
                "SELECT limited_day IS NOT NULL FROM ai_state WHERE slot=1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map(|v| v.unwrap_or(false))
            .map_err(failed)
    }
}
