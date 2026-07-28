/// Build the canonical `job_key` label from a job kind and a timeframe.
///
/// The format is `"{job_kind}-{timeframe}"` and intentionally excludes
/// `agent_key` so the same label can be used to refer to the same
/// logical job across many agents. Validation of the inputs is the
/// caller's responsibility; this helper is a pure formatting function.
pub fn build_generated_job_key(job_kind: &str, timeframe: &str) -> String {
    let job_kind = normalize_timeframe_for_job_key(job_kind);
    let timeframe = normalize_timeframe_for_job_key(timeframe);
    format!("{}-{timeframe}", job_kind.replace('_', "-"))
}

pub fn build_generated_event_job_key(job_kind: &str) -> String {
    job_kind.trim().replace('_', "-")
}

/// Trim a timeframe (or job kind) for inclusion in a generated key.
///
/// The returned slice is the input with leading and trailing whitespace
/// removed. Callers must guarantee the trimmed value is non-empty
/// before passing it here.
pub fn normalize_timeframe_for_job_key(value: &str) -> &str {
    value.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_generated_job_key_joins_kind_and_timeframe() {
        assert_eq!(build_generated_job_key("analysis", "15m"), "analysis-15m");
        assert_eq!(build_generated_job_key("trading", "1m"), "trading-1m");
        assert_eq!(build_generated_job_key("analysis", "1h"), "analysis-1h");
        assert_eq!(
            build_generated_job_key("daily_review", "1d"),
            "daily-review-1d"
        );
    }

    #[test]
    fn build_generated_job_key_trims_whitespace() {
        assert_eq!(
            build_generated_job_key("  analysis ", " 15m "),
            "analysis-15m"
        );
    }

    #[test]
    fn build_generated_job_key_does_not_include_agent_key() {
        let key = build_generated_job_key("analysis", "15m");
        assert!(!key.contains("agent"));
    }

    #[test]
    fn build_generated_event_job_key_replaces_underscores() {
        assert_eq!(
            build_generated_event_job_key("market_analysis"),
            "market-analysis"
        );
    }
}
