use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, TimeZone, Utc};

/// Default trigger delay (in seconds) applied after each candle boundary
/// before a scheduled run is considered due.
pub const DEFAULT_TRIGGER_DELAY_SECONDS: i32 = 1;

/// Parse a timeframe string into a duration in whole seconds.
///
/// Accepted format: a positive integer followed by exactly one of
/// `m` (minutes), `h` (hours), or `d` (days). Whitespace around the
/// value is ignored. Examples: `1m`, `15m`, `1h`, `4h`, `1d`, `3m`.
///
/// Rejects empty values, `0m`, negative integers, missing units,
/// unknown units such as `15s`, and overflows.
pub fn parse_timeframe_seconds(timeframe: &str) -> Result<i64> {
    let trimmed = timeframe.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("timeframe must not be empty"));
    }

    let (number_part, unit_part) = trimmed.split_at(trimmed.len() - 1);
    if unit_part.chars().count() != 1 {
        return Err(anyhow!(
            "timeframe must end with a single unit character (m, h, or d)"
        ));
    }
    if number_part.is_empty() {
        return Err(anyhow!("timeframe must include a number before the unit"));
    }

    let magnitude: i64 = number_part
        .parse()
        .map_err(|_| anyhow!("timeframe number must be a positive integer"))?;
    if magnitude <= 0 {
        return Err(anyhow!("timeframe number must be greater than zero"));
    }

    let seconds: i64 = match unit_part {
        "m" => magnitude
            .checked_mul(60)
            .ok_or_else(|| anyhow!("timeframe minutes overflowed i64"))?,
        "h" => magnitude
            .checked_mul(60 * 60)
            .ok_or_else(|| anyhow!("timeframe hours overflowed i64"))?,
        "d" => magnitude
            .checked_mul(60 * 60 * 24)
            .ok_or_else(|| anyhow!("timeframe days overflowed i64"))?,
        other => return Err(anyhow!("unsupported timeframe unit: {other}")),
    };

    Ok(seconds)
}

/// Parse a humanized timeout string into a duration in whole seconds.
///
/// Accepts the same units as [`parse_timeframe_seconds`] (`Nm`, `Nh`, `Nd`)
/// plus a seconds suffix (`Ns`), whitespace-separated composites like
/// `1h 30m`, and bare integers of seconds (e.g. `900`). Whitespace is
/// ignored between parts; the value must be strictly positive.
///
/// Rejects empty values, negative integers, zero, unknown units,
/// non-integer magnitudes, and overflows.
pub fn parse_timeout_seconds(raw: &str) -> Result<i64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("timeout must not be empty"));
    }

    if let Ok(value) = trimmed.parse::<i64>() {
        if value <= 0 {
            return Err(anyhow!("timeout must be greater than zero"));
        }
        return Ok(value);
    }

    let mut total: i64 = 0;
    let mut matched_any = false;
    for part in trimmed.split_whitespace() {
        let (number_part, unit_part) = part.split_at(part.len() - 1);
        if unit_part.chars().count() != 1 {
            return Err(anyhow!(
                "timeout part {part:?} must end with a single unit character (s, m, h, or d)"
            ));
        }
        if number_part.is_empty() {
            return Err(anyhow!("timeout part {part:?} must include a number before the unit"));
        }
        let magnitude: i64 = number_part
            .parse()
            .map_err(|_| anyhow!("timeout part {part:?} must be a positive integer"))?;
        if magnitude <= 0 {
            return Err(anyhow!("timeout part {part:?} must be greater than zero"));
        }
        let seconds: i64 = match unit_part {
            "s" => magnitude,
            "m" => magnitude
                .checked_mul(60)
                .ok_or_else(|| anyhow!("timeout minutes overflowed i64"))?,
            "h" => magnitude
                .checked_mul(60 * 60)
                .ok_or_else(|| anyhow!("timeout hours overflowed i64"))?,
            "d" => magnitude
                .checked_mul(60 * 60 * 24)
                .ok_or_else(|| anyhow!("timeout days overflowed i64"))?,
            other => return Err(anyhow!("unsupported timeout unit: {other}")),
        };
        total = total
            .checked_add(seconds)
            .ok_or_else(|| anyhow!("timeout overflowed i64"))?;
        matched_any = true;
    }

    if !matched_any {
        return Err(anyhow!("timeout must include at least one duration"));
    }
    if total <= 0 {
        return Err(anyhow!("timeout must be greater than zero"));
    }

    Ok(total)
}

/// Compute the first candle boundary (plus trigger delay) strictly after
/// `now` for the given timeframe.
///
/// Boundaries are aligned to the Unix epoch in UTC, so a `1m` timeframe
/// always aligns to whole UTC minutes and a `1d` timeframe always
/// aligns to UTC midnight.
pub fn next_due_after(
    now: DateTime<Utc>,
    timeframe: &str,
    trigger_delay_seconds: i32,
) -> Result<DateTime<Utc>> {
    let duration = parse_timeframe_seconds(timeframe)
        .with_context(|| format!("invalid timeframe {timeframe:?}"))?;
    if duration <= 0 {
        return Err(anyhow!("timeframe duration must be positive"));
    }
    if trigger_delay_seconds < 0 {
        return Err(anyhow!("trigger delay must be non-negative"));
    }

    let now_ts = now.timestamp();
    let boundary_ts = floor_div(now_ts, duration) * duration;
    let next_boundary_ts = boundary_ts
        .checked_add(duration)
        .ok_or_else(|| anyhow!("next boundary timestamp overflowed i64"))?;
    let next_due_ts = next_boundary_ts
        .checked_add(trigger_delay_seconds as i64)
        .ok_or_else(|| anyhow!("next due timestamp overflowed i64"))?;

    Utc.timestamp_opt(next_due_ts, 0)
        .single()
        .ok_or_else(|| anyhow!("next due DateTime out of range"))
}

/// Compute the latest candle boundary (plus trigger delay) that is
/// already due at or before `now`. Returns `None` when no boundary has
/// yet been reached.
pub fn latest_due_at_or_before(
    now: DateTime<Utc>,
    timeframe: &str,
    trigger_delay_seconds: i32,
) -> Result<Option<DateTime<Utc>>> {
    let duration = parse_timeframe_seconds(timeframe)
        .with_context(|| format!("invalid timeframe {timeframe:?}"))?;
    if duration <= 0 {
        return Err(anyhow!("timeframe duration must be positive"));
    }
    if trigger_delay_seconds < 0 {
        return Err(anyhow!("trigger delay must be non-negative"));
    }

    let now_ts = now.timestamp();
    let delay = trigger_delay_seconds as i64;
    if now_ts < delay {
        return Ok(None);
    }

    let anchor_ts = now_ts - delay;
    let latest_boundary_ts = floor_div(anchor_ts, duration) * duration;
    let latest_due_ts = latest_boundary_ts
        .checked_add(delay)
        .ok_or_else(|| anyhow!("latest due timestamp overflowed i64"))?;

    if latest_due_ts > now_ts {
        return Ok(None);
    }

    Ok(Utc
        .timestamp_opt(latest_due_ts, 0)
        .single()
        .map(|dt| dt.with_timezone(&Utc)))
}

/// Return the candle boundary associated with the given due instant.
/// For example, with a 1-second trigger delay, a due instant of
/// `12:00:01` maps back to the `12:00:00` boundary.
pub fn boundary_for_due_at(due_at: DateTime<Utc>, trigger_delay_seconds: i32) -> DateTime<Utc> {
    let delay = trigger_delay_seconds.max(0) as i64;
    let due_ts = due_at.timestamp();
    let boundary_ts = due_ts - delay;
    Utc.timestamp_opt(boundary_ts, 0).single().unwrap_or(due_at)
}

fn floor_div(numerator: i64, denominator: i64) -> i64 {
    let mut result = numerator / denominator;
    let remainder = numerator % denominator;
    if remainder < 0 {
        result -= 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).single().expect("valid ts")
    }

    #[test]
    fn parse_accepts_supported_units() {
        assert_eq!(parse_timeframe_seconds("1m").unwrap(), 60);
        assert_eq!(parse_timeframe_seconds("5m").unwrap(), 300);
        assert_eq!(parse_timeframe_seconds("15m").unwrap(), 900);
        assert_eq!(parse_timeframe_seconds("1h").unwrap(), 3600);
        assert_eq!(parse_timeframe_seconds("4h").unwrap(), 4 * 3600);
        assert_eq!(parse_timeframe_seconds("1d").unwrap(), 24 * 3600);
        assert_eq!(parse_timeframe_seconds("3m").unwrap(), 180);
        assert_eq!(parse_timeframe_seconds("2h").unwrap(), 7200);
        assert_eq!(parse_timeframe_seconds("7d").unwrap(), 7 * 24 * 3600);
    }

    #[test]
    fn parse_trims_whitespace() {
        assert_eq!(parse_timeframe_seconds("  15m  ").unwrap(), 900);
    }

    #[test]
    fn parse_rejects_invalid_values() {
        assert!(parse_timeframe_seconds("").is_err());
        assert!(parse_timeframe_seconds("   ").is_err());
        assert!(parse_timeframe_seconds("0m").is_err());
        assert!(parse_timeframe_seconds("0h").is_err());
        assert!(parse_timeframe_seconds("-1m").is_err());
        assert!(parse_timeframe_seconds("abc").is_err());
        assert!(parse_timeframe_seconds("15s").is_err());
        assert!(parse_timeframe_seconds("m").is_err());
        assert!(parse_timeframe_seconds("1").is_err());
        assert!(parse_timeframe_seconds("1mm").is_err());
        assert!(parse_timeframe_seconds("1.5h").is_err());
    }

    #[test]
    fn next_due_after_1m_aligns_to_minute_boundary() {
        let now = at(37);
        let next = next_due_after(now, "1m", 1).unwrap();
        assert_eq!(next, at(61));
    }

    #[test]
    fn next_due_after_15m_aligns_to_quarter_hour() {
        let now = at(12 * 3600 + 7 * 60);
        let next = next_due_after(now, "15m", 1).unwrap();
        assert_eq!(next, at(12 * 3600 + 15 * 60 + 1));
    }

    #[test]
    fn next_due_after_1h_aligns_to_hour() {
        let now = at(12 * 3600 + 17 * 60);
        let next = next_due_after(now, "1h", 1).unwrap();
        assert_eq!(next, at(13 * 3600 + 1));
    }

    #[test]
    fn next_due_after_1d_aligns_to_midnight() {
        let now = at(12 * 3600 + 30 * 60);
        let next = next_due_after(now, "1d", 1).unwrap();
        assert_eq!(next, at(24 * 3600 + 1));
    }

    #[test]
    fn next_due_after_zero_delay_returns_pure_boundary() {
        let now = at(12 * 3600 + 7 * 60 + 5);
        let next = next_due_after(now, "15m", 0).unwrap();
        assert_eq!(next, at(12 * 3600 + 15 * 60));
    }

    #[test]
    fn next_due_after_rejects_negative_delay() {
        let now = at(0);
        assert!(next_due_after(now, "1m", -1).is_err());
    }

    #[test]
    fn latest_due_at_or_before_returns_none_before_first_due() {
        let now = at(0);
        assert!(latest_due_at_or_before(now, "1m", 1).unwrap().is_none());
    }

    #[test]
    fn latest_due_at_or_before_finds_exact_due_instant() {
        let now = at(12 * 3600 + 1);
        let due = latest_due_at_or_before(now, "1m", 1).unwrap().unwrap();
        assert_eq!(due, at(12 * 3600 + 1));
    }

    #[test]
    fn latest_due_at_or_before_returns_latest_boundary_in_past() {
        let now = at(12 * 3600 + 34 * 60 + 50);
        let due = latest_due_at_or_before(now, "1m", 1).unwrap().unwrap();
        assert_eq!(due, at(12 * 3600 + 34 * 60 + 1));
    }

    #[test]
    fn latest_due_at_or_before_returns_none_within_delay_window() {
        let now = at(0);
        let due = latest_due_at_or_before(now, "1m", 1).unwrap();
        assert!(due.is_none());
    }

    #[test]
    fn latest_due_at_or_before_handles_zero_delay() {
        let now = at(60);
        let due = latest_due_at_or_before(now, "1m", 0).unwrap().unwrap();
        assert_eq!(due, at(60));
    }

    #[test]
    fn boundary_for_due_at_subtracts_delay() {
        let due = at(12 * 3600 + 1);
        assert_eq!(boundary_for_due_at(due, 1), at(12 * 3600));
        assert_eq!(boundary_for_due_at(due, 0), at(12 * 3600 + 1));
    }

    #[test]
    fn parse_timeout_accepts_timeframe_units() {
        assert_eq!(parse_timeout_seconds("1m").unwrap(), 60);
        assert_eq!(parse_timeout_seconds("15m").unwrap(), 900);
        assert_eq!(parse_timeout_seconds("1h").unwrap(), 3600);
        assert_eq!(parse_timeout_seconds("2h").unwrap(), 7200);
        assert_eq!(parse_timeout_seconds("1d").unwrap(), 24 * 3600);
    }

    #[test]
    fn parse_timeout_accepts_seconds() {
        assert_eq!(parse_timeout_seconds("30s").unwrap(), 30);
        assert_eq!(parse_timeout_seconds("90s").unwrap(), 90);
    }

    #[test]
    fn parse_timeout_accepts_composites() {
        assert_eq!(parse_timeout_seconds("1h 30m").unwrap(), 3600 + 30 * 60);
        assert_eq!(parse_timeout_seconds("2h 15m 30s").unwrap(), 2 * 3600 + 15 * 60 + 30);
        assert_eq!(parse_timeout_seconds("  1h   30m  ").unwrap(), 3600 + 30 * 60);
    }

    #[test]
    fn parse_timeout_accepts_bare_seconds() {
        assert_eq!(parse_timeout_seconds("900").unwrap(), 900);
        assert_eq!(parse_timeout_seconds("  120 ").unwrap(), 120);
    }

    #[test]
    fn parse_timeout_rejects_invalid_values() {
        assert!(parse_timeout_seconds("").is_err());
        assert!(parse_timeout_seconds("   ").is_err());
        assert!(parse_timeout_seconds("0").is_err());
        assert!(parse_timeout_seconds("0m").is_err());
        assert!(parse_timeout_seconds("0h").is_err());
        assert!(parse_timeout_seconds("-1m").is_err());
        assert!(parse_timeout_seconds("-30").is_err());
        assert!(parse_timeout_seconds("abc").is_err());
        assert!(parse_timeout_seconds("m").is_err());
        assert!(parse_timeout_seconds("1mm").is_err());
        assert!(parse_timeout_seconds("1.5h").is_err());
        assert!(parse_timeout_seconds("5x").is_err());
    }
}
