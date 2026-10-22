//! Content-free usage accounting. The worker alone owns the writer; node
//! readers return normalized calls and never open this store.

pub mod ai_usage;
pub mod prices;
pub mod project_cost;
pub mod query;
pub mod store;
pub mod worker;

pub const DAY_MS: u64 = 86_400_000;
pub const EVENT_DAYS: u64 = 45;
pub const DAILY_DAYS: i64 = 400;
pub const EVENT_LIMIT: usize = 1_000_000;
pub const DAILY_LIMIT: usize = 100_000;

/// A receipt's day is fixed at first ingestion in the core's time zone.
/// Moving the core or changing its zone affects new calls only.
pub fn local_day(at_ms: u64, zone: &jiff::tz::TimeZone) -> Result<String, &'static str> {
    let at = i64::try_from(at_ms).map_err(|_| "usage_timestamp_invalid")?;
    let at = jiff::Timestamp::from_millisecond(at).map_err(|_| "usage_timestamp_invalid")?;
    Ok(at.to_zoned(zone.clone()).date().to_string())
}
