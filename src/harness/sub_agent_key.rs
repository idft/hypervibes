/// Build the canonical `sub_agent_key` label from a job kind and a timeframe.
///
/// The format is `"{sub_agent_kind}-{timeframe}"` and intentionally excludes
/// `agent_key` so the same label can be used to refer to the same
/// logical job across many agents. Validation of the inputs is the
/// caller's responsibility; this helper is a pure formatting function.
pub fn build_generated_sub_agent_key(sub_agent_kind: &str, timeframe: &str) -> String {
    let sub_agent_kind = normalize_timeframe_for_sub_agent_key(sub_agent_kind);
    let timeframe = normalize_timeframe_for_sub_agent_key(timeframe);
    format!("{}-{timeframe}", sub_agent_kind.replace('_', "-"))
}

pub fn build_generated_event_sub_agent_key(sub_agent_kind: &str) -> String {
    sub_agent_kind.trim().replace('_', "-")
}

/// Trim a timeframe (or job kind) for inclusion in a generated key.
///
/// The returned slice is the input with leading and trailing whitespace
/// removed. Callers must guarantee the trimmed value is non-empty
/// before passing it here.
pub fn normalize_timeframe_for_sub_agent_key(value: &str) -> &str {
    value.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_generated_sub_agent_key_joins_kind_and_timeframe() {
        assert_eq!(
            build_generated_sub_agent_key("analysis", "15m"),
            "analysis-15m"
        );
        assert_eq!(build_generated_sub_agent_key("trading", "1m"), "trading-1m");
        assert_eq!(
            build_generated_sub_agent_key("analysis", "1h"),
            "analysis-1h"
        );
        assert_eq!(
            build_generated_sub_agent_key("daily_review", "1d"),
            "daily-review-1d"
        );
    }

    #[test]
    fn build_generated_sub_agent_key_trims_whitespace() {
        assert_eq!(
            build_generated_sub_agent_key("  analysis ", " 15m "),
            "analysis-15m"
        );
    }

    #[test]
    fn build_generated_sub_agent_key_does_not_include_agent_key() {
        let key = build_generated_sub_agent_key("analysis", "15m");
        assert!(!key.contains("agent"));
    }

    #[test]
    fn build_generated_event_sub_agent_key_replaces_underscores() {
        assert_eq!(
            build_generated_event_sub_agent_key("market_analysis"),
            "market-analysis"
        );
    }
}
