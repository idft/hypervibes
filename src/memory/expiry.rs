use chrono::{DateTime, Duration, Utc};

use crate::memory::MemoryRecord;

/// Compute the absolute expiration timestamp for a memory row, if it has
/// one. The rules (in priority order) are:
///
/// 1. `metadata.stale_after` — an absolute RFC 3339 timestamp. The backend
///    stamps it for new Analysis publications; older rows may be producer-set.
/// 2. `metadata.valid_for_seconds` — a relative duration added to
///    `created_at`.
/// 3. Legacy provenance-bearing research rows fall back to their memory
///    timeframe (15m => 30m, 1h => 2h, 1d => 48h, others => 30m).
///    New Analysis publications always materialize `stale_after` at insertion:
///    creation time plus two cycles of the producing run's schedule, overriding
///    caller validity; historical rows retain their original computation.
///
/// Returns `None` when the row has no implicit or explicit expiration
/// (e.g. an `observation` memory with no `valid_for_seconds`).
pub fn memory_expires_at(row: &MemoryRecord) -> Option<DateTime<Utc>> {
    stale_after(&row.metadata)
        .or_else(|| {
            valid_for_seconds(&row.metadata)
                .and_then(Duration::try_seconds)
                .and_then(|duration| row.created_at.checked_add_signed(duration))
        })
        .or_else(|| {
            // Only provenance-bearing research rows get a legacy timeframe
            // default; framework log types (trading decisions, reviews,
            // learnings) are durable audit records and never implicitly expire.
            if is_analysis_originated(row) {
                Some(
                    row.created_at
                        + analysis_default_valid_for(row.timeframe.as_deref().unwrap_or("")),
                )
            } else {
                None
            }
        })
}

/// Research rows published by an analysis producer. The provenance column
/// names the source run; rows created before provenance existed or by
/// non-analysis producers only expire when they provide explicit validity.
fn is_analysis_originated(row: &MemoryRecord) -> bool {
    row.source_run_id.is_some()
        && !matches!(
            row.memory_type.as_str(),
            "trading_decision" | "review" | "agent_learnings"
        )
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

/// Default validity for an analysis-produced research memory that has no
/// explicit `valid_for_seconds` or `stale_after` in its metadata.
///
/// Preserve this legacy mapping for historical audit reproducibility.
fn analysis_default_valid_for(timeframe: &str) -> Duration {
    match timeframe {
        "15m" => Duration::minutes(30),
        "1h" => Duration::minutes(120),
        "1d" => Duration::hours(48),
        _ => Duration::minutes(30),
    }
}

pub(crate) fn analysis_schedule_expires_at(
    schedule: Option<&str>,
    created_at: DateTime<Utc>,
) -> anyhow::Result<DateTime<Utc>> {
    let seconds = match schedule {
        Some(schedule) => crate::harness::timeframe::parse_timeframe_seconds(schedule)?
            .checked_mul(2)
            .ok_or_else(|| anyhow::anyhow!("analysis validity overflow"))?,
        None => 30 * 60,
    };
    Duration::try_seconds(seconds)
        .and_then(|duration| created_at.checked_add_signed(duration))
        .ok_or_else(|| anyhow::anyhow!("analysis expiration timestamp overflow"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_validity_uses_creation_time_and_canonical_parser() {
        let created_at = Utc::now();
        for (schedule, minutes) in [("5m", 10), ("15m", 30), ("4h", 480)] {
            assert_eq!(
                analysis_schedule_expires_at(Some(schedule), created_at).expect("valid schedule"),
                created_at + Duration::minutes(minutes),
            );
        }
        assert_eq!(
            analysis_schedule_expires_at(None, created_at).expect("fallback"),
            created_at + Duration::minutes(30),
        );
        assert_eq!(analysis_default_valid_for("5m"), Duration::minutes(30));
        assert_eq!(analysis_default_valid_for(""), Duration::minutes(30));
    }
}
