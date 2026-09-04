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

/// Validate a user-provided Analysis sub-agent key. The key is the durable
/// identity of a user-created Analysis job, so it must be a bounded ASCII
/// slug rather than a derived label.
pub fn is_valid_user_sub_agent_key(value: &str) -> bool {
    let value = value.trim();
    let (valid, length_ok) = {
        let length_ok = !value.is_empty() && value.len() <= 64;
        let valid = value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));
        (valid, length_ok)
    };
    valid && length_ok
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
        assert_eq!(build_generated_sub_agent_key("review", "1d"), "review-1d");
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
            build_generated_event_sub_agent_key("quant_research"),
            "quant-research"
        );
    }

    #[test]
    fn user_sub_agent_keys_accept_bounded_ascii_slugs() {
        assert!(is_valid_user_sub_agent_key("my-news-research"));
        assert!(is_valid_user_sub_agent_key("sentiment_15m"));
        assert!(is_valid_user_sub_agent_key("a"));
        assert!(is_valid_user_sub_agent_key(&"x".repeat(64)));
    }

    #[test]
    fn user_sub_agent_keys_reject_invalid_values() {
        assert!(!is_valid_user_sub_agent_key(""));
        assert!(!is_valid_user_sub_agent_key("   "));
        assert!(!is_valid_user_sub_agent_key(&"x".repeat(65)));
        assert!(!is_valid_user_sub_agent_key("has spaces"));
        assert!(!is_valid_user_sub_agent_key("no-symbols!"));
        assert!(!is_valid_user_sub_agent_key("ünicode"));
    }
}
