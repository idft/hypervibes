use chrono::{DateTime, Duration, Utc};

use crate::memory::MemoryRecord;

/// Compute the absolute expiration timestamp for a memory row, if it has
/// one. The rules (in priority order) are:
///
/// 1. `metadata.stale_after` — an explicit RFC 3339 timestamp set by the
///    producer when it knows the data goes stale at a wall-clock instant.
/// 2. `metadata.valid_for_seconds` — a relative duration added to
///    `created_at`.
/// 3. For `memory_type == "analysis"` rows with neither of the above,
///    fall back to the per-timeframe defaults documented in
///    [`analysis_default_valid_for`]. These are intentionally **2x the
///    schedule interval** so the trading loop has a one-cycle fallback if
///    the next analysis is delayed.
///
/// Returns `None` when the row has no implicit or explicit expiration
/// (e.g. an `observation` memory with no `valid_for_seconds`).
pub fn memory_expires_at(row: &MemoryRecord) -> Option<DateTime<Utc>> {
    stale_after(&row.metadata)
        .or_else(|| {
            valid_for_seconds(&row.metadata)
                .map(|seconds| row.created_at + Duration::seconds(seconds))
        })
        .or_else(|| {
            if row.memory_type == "analysis" {
                Some(
                    row.created_at
                        + analysis_default_valid_for(row.timeframe.as_deref().unwrap_or("")),
                )
            } else {
                None
            }
        })
}

fn valid_for_seconds(metadata: &serde_json::Value) -> Option<i64> {
    metadata
        .get("valid_for_seconds")
        .and_then(|value| match value {
            serde_json::Value::Number(number) => number
                .as_i64()
                .filter(|seconds| *seconds > 0)
                .or_else(|| {
                    number
                        .as_u64()
                        .and_then(|seconds| i64::try_from(seconds).ok())
                        .filter(|seconds| *seconds > 0)
                })
                .or_else(|| {
                    let seconds = number.as_f64()?;
                    if !seconds.is_finite() || seconds < 1.0 || seconds > i64::MAX as f64 {
                        return None;
                    }
                    Some(seconds.floor() as i64)
                }),
            _ => None,
        })
}

fn stale_after(metadata: &serde_json::Value) -> Option<DateTime<Utc>> {
    metadata
        .get("stale_after")
        .and_then(|value| value.as_str())
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

/// Default validity for an analysis memory that has no explicit
/// `valid_for_seconds` or `stale_after` in its metadata.
///
/// The values are intentionally **2x the schedule interval**: an analysis
/// loop is expected to run on every schedule tick, but the next analysis
/// may be delayed (model latency, provider 429s, a missed cron tick, etc).
/// Keeping the memory valid for a full second cycle gives the trading
/// loop a one-cycle fallback instead of going `[SILENT]` on the first
/// delay. New analyses still supersede older ones, so the longer window
/// adds tolerance, not stale signal.
fn analysis_default_valid_for(timeframe: &str) -> Duration {
    match timeframe {
        "15m" => Duration::minutes(30),
        "1h" => Duration::minutes(120),
        "1d" => Duration::hours(48),
        _ => Duration::minutes(30),
    }
}
