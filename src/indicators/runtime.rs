use std::{collections::BTreeMap, panic::AssertUnwindSafe, sync::Arc};

use anyhow::{Context, Result, anyhow, bail, ensure};
use pine_lang::{
    RunResult, ScriptBuilder,
    core::{
        Data, DefaultPineOutput, InputOutput, InputValue, MetadataOutput, Ohlcv, SymInfo, Timeframe,
    },
};
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Semaphore;

use super::model::{Candle, IndicatorDiagnostic, IndicatorMetadata};

pub const MAX_INDICATOR_SOURCE_BYTES: usize = 64 * 1024;
pub const MAX_INDICATOR_TARGETS: usize = 64;
pub const MAX_INDICATOR_HISTORY_BARS: usize = 2_000;
pub const DEFAULT_INDICATOR_HISTORY_BARS: usize = 500;
pub const MAX_INDICATOR_PLOTS: usize = 32;
pub const MAX_INDICATOR_INPUTS: usize = 32;
pub const MAX_INDICATOR_INPUT_TITLE_BYTES: usize = 128;
pub const MAX_INDICATOR_RESULT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_CONCURRENT_INDICATOR_EXECUTIONS: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IndicatorInputMetadata {
    pub kind: String,
    pub title: String,
    pub group: String,
    pub default: Value,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub step: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatedIndicatorSource {
    pub metadata: IndicatorMetadata,
    pub inputs: Vec<IndicatorInputMetadata>,
    pub diagnostics: Vec<IndicatorDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicatorExecutionOutput {
    pub metadata: IndicatorMetadata,
    pub plots: BTreeMap<String, Vec<Option<f64>>>,
    pub latest_values: BTreeMap<String, f64>,
    pub diagnostics: Vec<IndicatorDiagnostic>,
}

fn diagnostic_from_pine(diagnostic: &pine_lang::diagnostics::Diagnostic) -> IndicatorDiagnostic {
    IndicatorDiagnostic {
        severity: diagnostic.severity.to_string(),
        line: diagnostic.line().map(|line| line as usize),
        column: diagnostic.column().map(|column| column as usize),
        message: diagnostic.message.clone(),
    }
}

fn input_value_to_json(value: &InputValue) -> Result<Value> {
    match value {
        InputValue::Bool(value) => Ok(Value::Bool(*value)),
        InputValue::Int(value) => Ok(Value::from(*value)),
        InputValue::Float(value) if value.is_finite() => Ok(Value::from(*value)),
        InputValue::Str(value) => Ok(Value::String(value.clone())),
        InputValue::Color(_) => bail!("color inputs are not supported by HyperVibes indicators"),
        InputValue::Float(_) => bail!("input default must be finite"),
    }
}

fn source_without_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
        .collect::<Vec<_>>()
        .join("\n")
}

fn validate_input_values(inputs: &[IndicatorInputMetadata], values: &Value) -> Result<()> {
    let values = values
        .as_object()
        .ok_or_else(|| anyhow!("indicator input values must be a JSON object"))?;
    for title in values.keys() {
        ensure!(
            inputs.iter().any(|input| input.title == *title),
            "unknown indicator input '{title}'"
        );
    }
    for input in inputs {
        let Some(value) = values.get(&input.title) else {
            continue;
        };
        let numeric = match input.kind.as_str() {
            "bool" => {
                ensure!(
                    value.is_boolean(),
                    "input '{}' must be boolean",
                    input.title
                );
                None
            }
            "int" => Some(
                value
                    .as_i64()
                    .ok_or_else(|| anyhow!("input '{}' must be an integer", input.title))?
                    as f64,
            ),
            "float" => Some(
                value
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| anyhow!("input '{}' must be a finite number", input.title))?,
            ),
            "string" => {
                ensure!(
                    value.is_string(),
                    "input '{}' must be a string",
                    input.title
                );
                None
            }
            "source" => {
                ensure!(
                    matches!(
                        value.as_str(),
                        Some("open" | "high" | "low" | "close" | "hl2" | "hlc3" | "ohlc4")
                    ),
                    "input '{}' must be a supported price source",
                    input.title
                );
                None
            }
            kind => bail!("input.{kind} is not supported by HyperVibes indicators"),
        };
        if let Some(value) = numeric {
            if let Some(minimum) = input.min_value {
                ensure!(
                    value >= minimum,
                    "input '{}' is below its minimum",
                    input.title
                );
            }
            if let Some(maximum) = input.max_value {
                ensure!(
                    value <= maximum,
                    "input '{}' is above its maximum",
                    input.title
                );
            }
        }
    }
    Ok(())
}

/// Compile Pine source and derive the supported metadata without giving it any
/// host capabilities beyond the builtin language surface.
pub fn validate_indicator_source(
    source: &str,
    input_values: &Value,
) -> Result<ValidatedIndicatorSource> {
    ensure!(
        !source.trim().is_empty(),
        "indicator source must not be blank"
    );
    ensure!(
        source.len() <= MAX_INDICATOR_SOURCE_BYTES,
        "indicator source exceeds {MAX_INDICATOR_SOURCE_BYTES} bytes"
    );
    let stripped = source_without_comments(source);
    for prohibited in ["import ", "request.", "strategy(", "library("] {
        ensure!(
            !stripped.contains(prohibited),
            "{prohibited} is not supported by HyperVibes indicators"
        );
    }
    ensure!(
        stripped.contains("indicator("),
        "Pine source must declare indicator(...); strategies and libraries are not supported"
    );
    let diagnostics = pine_lang::check(source, None).map_err(|error| anyhow!(error.to_string()))?;
    let diagnostics: Vec<_> = diagnostics.iter().map(diagnostic_from_pine).collect();
    ensure!(
        !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == "error"),
        "Pine source has validation errors"
    );
    let decoded = pine_lang::decode_metadata::<DefaultPineOutput>(source, Default::default())
        .map_err(|error| anyhow!(error.to_string()))?;
    let indicator = decoded
        .indicator()
        .ok_or_else(|| anyhow!("Pine source must declare indicator(...)"))?;
    ensure!(
        !indicator.title.trim().is_empty(),
        "indicator declaration must include a nonblank title"
    );
    ensure!(
        decoded.inputs().len() <= MAX_INDICATOR_INPUTS,
        "indicator declares more than {MAX_INDICATOR_INPUTS} inputs"
    );
    let mut input_titles = std::collections::BTreeSet::new();
    let inputs = decoded
        .inputs()
        .iter()
        .map(|input| {
            ensure!(
                !input.title.trim().is_empty()
                    && input.title.len() <= MAX_INDICATOR_INPUT_TITLE_BYTES,
                "indicator input titles must be 1-{MAX_INDICATOR_INPUT_TITLE_BYTES} bytes"
            );
            ensure!(
                input_titles.insert(input.title.clone()),
                "indicator input titles must be unique"
            );
            Ok(IndicatorInputMetadata {
                kind: input.kind.clone(),
                title: input.title.clone(),
                group: input.group.clone(),
                default: input_value_to_json(&input.default)?,
                min_value: input.min_val,
                max_value: input.max_val,
                step: input.step,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    validate_input_values(&inputs, input_values)?;
    let plot_count = stripped.matches("plot(").count();
    ensure!(
        plot_count <= MAX_INDICATOR_PLOTS,
        "indicator declares more than {MAX_INDICATOR_PLOTS} plots"
    );
    Ok(ValidatedIndicatorSource {
        metadata: IndicatorMetadata {
            name: indicator.title.clone(),
            overlay: indicator.overlay,
            plot_count,
        },
        inputs,
        diagnostics,
    })
}

fn pine_timeframe(timeframe: &str) -> Result<Timeframe> {
    let seconds = crate::harness::timeframe::parse_timeframe_seconds(timeframe)?;
    let milliseconds = seconds
        .checked_mul(1_000)
        .ok_or_else(|| anyhow!("timeframe is too large"))?;
    Timeframe::from_millis(milliseconds)
        .ok_or_else(|| anyhow!("unsupported Pine timeframe {timeframe}"))
}

fn execute_blocking(
    source: &str,
    input_values: &Value,
    candles: &[Candle],
    symbol: &str,
    timeframe: &str,
) -> Result<IndicatorExecutionOutput> {
    let validated = validate_indicator_source(source, input_values)?;
    ensure!(!candles.is_empty(), "indicator execution requires candles");
    ensure!(
        candles.len() <= MAX_INDICATOR_HISTORY_BARS,
        "indicator execution exceeds {MAX_INDICATOR_HISTORY_BARS} candles"
    );
    let rows = candles
        .iter()
        .map(|candle| {
            let to_f64 = |value: rust_decimal::Decimal, field: &str| {
                value
                    .to_f64()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| anyhow!("candle {field} is not finite"))
            };
            Ok(Ohlcv {
                time: candle.opened_at.timestamp_millis(),
                open: to_f64(candle.open, "open")?,
                high: to_f64(candle.high, "high")?,
                low: to_f64(candle.low, "low")?,
                close: to_f64(candle.close, "close")?,
                volume: to_f64(candle.volume, "volume")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let inputs = pine_lang::inputs_from_json(&input_values.to_string())
        .context("failed to decode indicator input values")?;
    let data = Data::from_ohlcv(rows).with_syminfo(SymInfo {
        ticker: symbol.to_string(),
        tickerid: format!("HYPERLIQUID:{symbol}"),
        description: symbol.to_string(),
        prefix: "HYPERLIQUID".to_string(),
        currency: "USD".to_string(),
        basecurrency: symbol.to_string(),
        type_: "crypto".to_string(),
        ..Default::default()
    });
    let run = ScriptBuilder::<DefaultPineOutput>::with_code(source)
        .with_data(data)
        .with_inputs(inputs)
        .with_timeframe(pine_timeframe(timeframe)?)
        .compile()
        .map_err(|error| anyhow!(error.to_string()))?
        .run()
        .map_err(|error| anyhow!(error.to_string()))?;
    let result = RunResult::collect(&run.outputs);
    ensure!(
        result.bars == candles.len(),
        "Pine runtime returned a misaligned bar count"
    );
    for (title, values) in &result.plots {
        ensure!(
            values.len() == candles.len(),
            "plot '{title}' is not aligned to candles"
        );
        ensure!(
            values.iter().flatten().all(|value| value.is_finite()),
            "plot '{title}' returned a non-finite value"
        );
    }
    Ok(IndicatorExecutionOutput {
        metadata: validated.metadata,
        latest_values: latest_values(&result.plots)?,
        plots: result.plots,
        diagnostics: validated.diagnostics,
    })
}

/// Run a validated indicator on a bounded blocking worker. The interpreter is
/// not process-isolated; the semaphore limits how much CPU a hostile script can consume.
pub async fn execute_indicator(
    execution_limit: Arc<Semaphore>,
    source: String,
    input_values: Value,
    candles: Vec<Candle>,
    symbol: String,
    timeframe: String,
) -> Result<IndicatorExecutionOutput> {
    let permit = execution_limit
        .acquire_owned()
        .await
        .context("indicator execution limit closed")?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        std::panic::catch_unwind(AssertUnwindSafe(|| {
            execute_blocking(&source, &input_values, &candles, &symbol, &timeframe)
        }))
        .map_err(|_| anyhow!("Pine runtime panicked"))?
    })
    .await
    .context("indicator execution task failed")?
}

pub fn latest_values(plots: &BTreeMap<String, Vec<Option<f64>>>) -> Result<BTreeMap<String, f64>> {
    let mut values = BTreeMap::new();
    for (title, points) in plots {
        if let Some(value) = points.iter().rev().flatten().next() {
            ensure!(
                value.is_finite(),
                "plot '{title}' returned a non-finite value"
            );
            values.insert(title.clone(), *value);
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use rust_decimal::Decimal;

    use super::*;

    fn candles() -> Vec<Candle> {
        (1..=40)
            .map(|index| {
                let close = Decimal::from(index);
                Candle {
                    opened_at: Utc
                        .timestamp_millis_opt(i64::from(index) * 60_000)
                        .single()
                        .expect("timestamp"),
                    open: close - Decimal::ONE,
                    high: close + Decimal::ONE,
                    low: close - Decimal::ONE,
                    close,
                    volume: Decimal::from(100),
                }
            })
            .collect()
    }

    #[test]
    fn validates_pine_metadata_and_typed_input_overrides() {
        let source = "//@version=5\nindicator(\"SMA\", overlay=true)\nlength = input.int(20, \"Length\", minval=1)\nplot(ta.sma(close, length), \"SMA\")";
        let validated = validate_indicator_source(source, &serde_json::json!({"Length": 10}))
            .expect("validate source");
        assert!(validated.metadata.overlay);
        assert_eq!(validated.inputs[0].title, "Length");
        assert!(validate_indicator_source(source, &serde_json::json!({"Length": 0})).is_err());
        assert!(validate_indicator_source(source, &serde_json::json!({"Unknown": 1})).is_err());
    }

    #[tokio::test]
    async fn executes_sma_rsi_and_macd_against_closed_candles() {
        let semaphore = Arc::new(Semaphore::new(1));
        for (source, titles) in [
            (
                "//@version=5\nindicator(\"SMA\", overlay=true)\nplot(ta.sma(close, 3), \"SMA\")",
                &["SMA"][..],
            ),
            (
                "//@version=5\nindicator(\"RSI\", overlay=false)\nplot(ta.rsi(close, 14), \"RSI\")",
                &["RSI"][..],
            ),
            (
                "//@version=5\nindicator(\"MACD\", overlay=false)\n[line, signal, histogram] = ta.macd(close, 12, 26, 9)\nplot(line, \"MACD\")\nplot(signal, \"Signal\")\nplot(histogram, \"Histogram\")",
                &["MACD", "Signal", "Histogram"][..],
            ),
        ] {
            let output = execute_indicator(
                Arc::clone(&semaphore),
                source.to_string(),
                serde_json::json!({}),
                candles(),
                "BTC".to_string(),
                "1m".to_string(),
            )
            .await
            .expect("execute indicator");
            assert_eq!(output.plots.len(), titles.len());
            for title in titles {
                assert_eq!(output.plots[*title].len(), 40);
            }
        }
    }

    #[test]
    fn rejects_prohibited_or_malformed_source() {
        for source in [
            "strategy(\"x\")",
            "indicator(\"x\")\nrequest.security(\"BTC\", \"1h\", close)",
            "import foo/bar/1",
            "indicator(\"x\")\nplot(",
        ] {
            assert!(
                validate_indicator_source(source, &serde_json::json!({})).is_err(),
                "{source}"
            );
        }
    }
}
