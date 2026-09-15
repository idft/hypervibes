use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use super::model::{IndicatorDiagnostic, IndicatorMetadata};

pub const MAX_INDICATOR_SOURCE_BYTES: usize = 64 * 1024;
pub const MAX_INDICATOR_TARGETS: usize = 64;
pub const MAX_INDICATOR_HISTORY_BARS: usize = 2_000;
pub const DEFAULT_INDICATOR_HISTORY_BARS: usize = 500;
pub const MAX_INDICATOR_PLOTS: usize = 32;
pub const MAX_INDICATOR_RESULT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_CONCURRENT_INDICATOR_EXECUTIONS: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatedIndicatorSource {
    pub metadata: IndicatorMetadata,
    pub diagnostics: Vec<IndicatorDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicatorExecutionOutput {
    pub metadata: IndicatorMetadata,
    pub plots: BTreeMap<String, Vec<Option<f64>>>,
    pub latest_values: BTreeMap<String, f64>,
    pub diagnostics: Vec<IndicatorDiagnostic>,
}

/// Validate the supported Pine surface before a source can become a version.
/// Runtime compilation is deliberately performed separately on bounded candles.
pub fn validate_indicator_source(source: &str) -> Result<ValidatedIndicatorSource> {
    if source.trim().is_empty() {
        bail!("indicator source must not be blank");
    }
    if source.len() > MAX_INDICATOR_SOURCE_BYTES {
        bail!("indicator source exceeds {MAX_INDICATOR_SOURCE_BYTES} bytes");
    }
    let source_without_comments = source
        .lines()
        .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
        .collect::<Vec<_>>()
        .join("\n");
    for prohibited in ["import ", "request.", "strategy(", "library("] {
        if source_without_comments.contains(prohibited) {
            bail!("{prohibited} is not supported by HyperVibes indicators");
        }
    }
    let indicator_start = source_without_comments.find("indicator(").ok_or_else(|| {
        anyhow!(
            "Pine source must declare indicator(...); strategies and libraries are not supported"
        )
    })?;
    let declaration = &source_without_comments[indicator_start..];
    let name = declaration
        .split('"')
        .nth(1)
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| anyhow!("indicator declaration must include a nonblank title"))?
        .to_string();
    let plot_count = source_without_comments.matches("plot(").count();
    if plot_count > MAX_INDICATOR_PLOTS {
        bail!("indicator declares more than {MAX_INDICATOR_PLOTS} plots");
    }
    Ok(ValidatedIndicatorSource {
        metadata: IndicatorMetadata {
            name,
            overlay: declaration.contains("overlay=true") || declaration.contains("overlay = true"),
            plot_count,
        },
        diagnostics: Vec::new(),
    })
}

pub fn latest_values(plots: &BTreeMap<String, Vec<Option<f64>>>) -> Result<BTreeMap<String, f64>> {
    let mut values = BTreeMap::new();
    for (title, points) in plots {
        if let Some(value) = points.iter().rev().flatten().next() {
            if !value.is_finite() {
                bail!("plot '{title}' returned a non-finite value");
            }
            values.insert(title.clone(), *value);
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_supported_indicator_declaration() {
        let source =
            "//@version=5\nindicator(\"SMA\", overlay=true)\nplot(ta.sma(close, 20), \"SMA\")";
        let validated = validate_indicator_source(source).expect("valid source");
        assert!(validated.metadata.overlay);
        assert_eq!(validated.metadata.plot_count, 1);
    }

    #[test]
    fn rejects_unsafe_pine_features() {
        for source in [
            "strategy(\"x\")",
            "indicator(\"x\")\nrequest.security(\"BTC\", \"1h\", close)",
            "import foo/bar/1",
        ] {
            assert!(validate_indicator_source(source).is_err(), "{source}");
        }
    }
}
