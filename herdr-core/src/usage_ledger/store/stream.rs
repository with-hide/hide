//! Claude emits successive views of one message. Keep its field maxima,
//! never another call, and move its whole-call price band atomically.
use super::*;

fn subtract(target: &mut Tokens, old: &Tokens) -> Result<(), &'static str> {
    for (target, old) in [
        (&mut target.input, old.input),
        (&mut target.output, old.output),
        (&mut target.cache_read, old.cache_read),
        (&mut target.cache_write, old.cache_write),
        (&mut target.cache_write_1h, old.cache_write_1h),
    ] {
        *target = target.checked_sub(old).ok_or("usage_total_inconsistent")?;
    }
    if let Some(value) = old.reasoning {
        let n = target
            .reasoning
            .as_mut()
            .ok_or("usage_total_inconsistent")?;
        *n = n.checked_sub(value).ok_or("usage_total_inconsistent")?;
    }
    Ok(())
}

/// What a later view of a stored call did to it.
pub(super) enum Merged {
    /// It held no new maximum: nothing was written.
    Same,
    /// The call and its day's totals now hold the new maxima.
    Rewrote,
}

/// `None` when `next` is not a later view of `old` but another call under
/// the same key.
pub(super) fn merge(
    tx: &rusqlite::Transaction<'_>,
    device: &str,
    old: &Event,
    next: &Event,
    frozen_at: u64,
) -> Result<Option<Merged>, &'static str> {
    if old.at_ms < frozen_at
        || old.model != next.model
        || old.recorded_cost_micro_usd.is_some()
        || next.recorded_cost_micro_usd.is_some()
        || old.incomplete
        || next.incomplete
    {
        return Ok(None);
    }
    let mut merged = old.clone();
    for (value, next) in [
        (&mut merged.tokens.input, next.tokens.input),
        (&mut merged.tokens.output, next.tokens.output),
        (&mut merged.tokens.cache_read, next.tokens.cache_read),
        (&mut merged.tokens.cache_write, next.tokens.cache_write),
        (
            &mut merged.tokens.cache_write_1h,
            next.tokens.cache_write_1h,
        ),
    ] {
        *value = (*value).max(next);
    }
    merged.tokens.reasoning = merged.tokens.reasoning.max(next.tokens.reasoning);
    validate(&merged)?;
    if merged == *old {
        return Ok(Some(Merged::Same));
    }
    // Read the persisted original day: changing the machine's timezone must
    // not move an already accounted call into another rollup.
    let day: String = tx
        .query_row(
            "SELECT day FROM events WHERE device=?1 AND agent='claude' AND key=?2",
            params![device, old.key],
            |row| row.get(0),
        )
        .map_err(failed)?;
    let raw: String = tx
        .query_row(
            "SELECT totals FROM daily WHERE device=?1 AND agent='claude' AND day=?2 AND model=?3",
            params![device, day, old.model],
            |row| row.get(0),
        )
        .map_err(failed)?;
    let mut totals: Totals = decode(&raw)?;
    subtract(&mut totals.tokens, &old.tokens)?;
    let prompt = old
        .tokens
        .input
        .checked_add(old.tokens.cache_read)
        .and_then(|v| v.checked_add(old.tokens.cache_write))
        .ok_or("usage_total_overflow")?;
    subtract(
        &mut totals.price_bands[super::super::prices::prompt_band(prompt)],
        &old.tokens,
    )?;
    totals.calls = totals
        .calls
        .checked_sub(1)
        .ok_or("usage_total_inconsistent")?;
    totals.missing_cost_calls = totals
        .missing_cost_calls
        .checked_sub(1)
        .ok_or("usage_total_inconsistent")?;
    totals.add(&merged)?;
    tx.execute(
        "UPDATE daily SET totals=?4 WHERE device=?1 AND agent='claude' AND day=?2 AND model=?3",
        params![device, day, old.model, encode(&totals)?],
    )
    .map_err(failed)?;
    tx.execute(
        "UPDATE events SET event=?3,fingerprint=?4 WHERE device=?1 AND agent='claude' AND key=?2",
        params![device, old.key, encode(&merged)?, fingerprint(&merged)?],
    )
    .map_err(failed)?;
    Ok(Some(Merged::Rewrote))
}
