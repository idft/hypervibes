use std::{collections::BTreeMap, panic::AssertUnwindSafe, sync::Arc};

use anyhow::{Context, Result, anyhow, bail, ensure};
use pine_lang::{
    RunResult, ScriptBuilder,
    ast::{Expr, Stmt, Visitor, walk_expr, walk_stmt},
    core::{
        Color, Data, DefaultPineOutput, InputOutput, InputValue, MetadataOutput, Ohlcv,
        PineVersion, PlotOutput, SymInfo, Timeframe,
    },
    lexer::Lexer,
    parser::Parser,
};
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Semaphore;

use super::model::{
    Candle, INDICATOR_VISUAL_DATA_VERSION, IndicatorColor, IndicatorDiagnostic, IndicatorMarker,
    IndicatorMetadata, IndicatorVisualData,
};

pub const MAX_INDICATOR_SOURCE_BYTES: usize = 64 * 1024;
pub const MAX_INDICATOR_TARGETS: usize = 64;
pub const MAX_INDICATOR_HISTORY_BARS: usize = 2_000;
pub const DEFAULT_INDICATOR_HISTORY_BARS: usize = 500;
pub const MAX_INDICATOR_PLOTS: usize = 32;
pub const MAX_INDICATOR_INPUTS: usize = 32;
pub const MAX_INDICATOR_INPUT_TITLE_BYTES: usize = 128;
pub const MAX_INDICATOR_MARKER_CALLS: usize = 32;
pub const MAX_INDICATOR_MARKER_TITLE_BYTES: usize = 128;
pub const MAX_INDICATOR_MARKER_TEXT_BYTES: usize = 256;
pub const MAX_INDICATOR_MARKER_CHARACTER_BYTES: usize = 32;
pub const MAX_INDICATOR_RESULT_BYTES: usize = 2 * 1024 * 1024;

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
    pub visual_data: IndicatorVisualData,
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

fn inspect_source(source: &str) -> Result<()> {
    #[derive(Default)]
    struct SourceInspector {
        found_loop: bool,
        marker_calls: usize,
    }

    impl Visitor for SourceInspector {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            if matches!(
                stmt,
                Stmt::For { .. } | Stmt::ForIn { .. } | Stmt::While { .. }
            ) {
                self.found_loop = true;
            }
            walk_stmt(self, stmt);
        }

        fn visit_expr(&mut self, expr: &Expr) {
            if let Expr::Call { callee, .. } = expr
                && let Expr::Variable { name, .. } = callee.as_ref()
                && matches!(name.as_str(), "plotshape" | "plotchar" | "plotarrow")
            {
                self.marker_calls = self.marker_calls.saturating_add(1);
            }
            walk_expr(self, expr);
        }
    }

    let version = PineVersion::detect(source)
        .map_err(|error| anyhow!(error.to_string()))?
        .unwrap_or(PineVersion::LATEST);
    let tokens = Lexer::with_version(source, version)
        .tokenize()
        .map_err(|error| anyhow!(error.to_string()))?;
    let program = Parser::new(tokens)
        .parse_program()
        .map_err(|error| anyhow!(error.to_string()))?;
    let mut inspector = SourceInspector::default();
    inspector.visit_program(&program);
    ensure!(
        !inspector.found_loop,
        "Pine loops are not supported by HyperVibes indicators"
    );
    ensure!(
        inspector.marker_calls <= MAX_INDICATOR_MARKER_CALLS,
        "indicator declares more than {MAX_INDICATOR_MARKER_CALLS} marker output calls"
    );
    Ok(())
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
    // Metadata decoding executes the AST, and this Pine runtime has no instruction budget.
    inspect_source(source)?;
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

fn indicator_color(color: &Color) -> IndicatorColor {
    IndicatorColor {
        red: color.r,
        green: color.g,
        blue: color.b,
        transparency: color.t,
    }
}

fn validate_marker_string(value: &str, limit: usize, field: &str) -> Result<()> {
    ensure!(
        value.len() <= limit,
        "indicator marker {field} exceeds {limit} bytes"
    );
    Ok(())
}

fn validate_marker_offset(offset: f64) -> Result<i64> {
    ensure!(offset.is_finite(), "indicator marker offset must be finite");
    ensure!(
        offset.fract() == 0.0,
        "indicator marker offset must be an integer"
    );
    ensure!(
        offset >= -(MAX_INDICATOR_HISTORY_BARS as f64)
            && offset <= MAX_INDICATOR_HISTORY_BARS as f64,
        "indicator marker offset exceeds supported history"
    );
    Ok(offset as i64)
}

fn validate_show_last(show_last: Option<f64>) -> Result<Option<f64>> {
    let Some(show_last) = show_last else {
        return Ok(None);
    };
    ensure!(
        show_last.is_finite() && show_last >= 0.0 && show_last.fract() == 0.0,
        "indicator marker show_last must be a nonnegative integer"
    );
    Ok(Some(show_last))
}

fn is_inside_show_last(bar_index: usize, bar_count: usize, show_last: Option<f64>) -> bool {
    match show_last {
        None => true,
        Some(0.0) => false,
        Some(value) if value >= bar_count as f64 => true,
        Some(value) => bar_index >= bar_count - value as usize,
    }
}

fn validate_marker_location(location: &str) -> Result<()> {
    ensure!(
        matches!(
            location,
            "abovebar" | "belowbar" | "top" | "bottom" | "absolute"
        ),
        "unsupported indicator marker location '{location}'"
    );
    Ok(())
}

fn validate_marker_size(size: &str) -> Result<()> {
    ensure!(
        matches!(
            size,
            "auto" | "tiny" | "small" | "normal" | "large" | "huge"
        ),
        "unsupported indicator marker size '{size}'"
    );
    Ok(())
}

fn validate_plotshape_style(style: &str) -> Result<()> {
    ensure!(
        matches!(
            style,
            "xcross"
                | "cross"
                | "circle"
                | "triangleup"
                | "triangledown"
                | "flag"
                | "arrowup"
                | "arrowdown"
                | "square"
                | "diamond"
                | "labelup"
                | "labeldown"
        ),
        "unsupported plotshape style '{style}'"
    );
    Ok(())
}

fn marker_is_active(value: f64, location: &str) -> Result<bool> {
    if value.is_nan() {
        return Ok(false);
    }
    ensure!(value.is_finite(), "indicator marker value must be finite");
    Ok(location == "absolute" || value != 0.0)
}

fn collect_visual_data(outputs: &[DefaultPineOutput]) -> Result<IndicatorVisualData> {
    let mut markers = Vec::new();
    for (bar_index, output) in outputs.iter().enumerate() {
        let marker_count = output
            .plotshapes()
            .len()
            .checked_add(output.plotchars().len())
            .and_then(|count| count.checked_add(output.plotarrows().len()))
            .ok_or_else(|| anyhow!("indicator marker output count overflowed"))?;
        ensure!(
            marker_count <= MAX_INDICATOR_MARKER_CALLS,
            "indicator emitted more than {MAX_INDICATOR_MARKER_CALLS} marker outputs on one candle"
        );

        for shape in output.plotshapes() {
            validate_marker_string(&shape.title, MAX_INDICATOR_MARKER_TITLE_BYTES, "title")?;
            validate_marker_string(&shape.text, MAX_INDICATOR_MARKER_TEXT_BYTES, "text")?;
            validate_plotshape_style(&shape.style)?;
            validate_marker_location(&shape.location)?;
            validate_marker_size(&shape.size)?;
            let offset = validate_marker_offset(shape.offset)?;
            let show_last = validate_show_last(shape.show_last)?;
            if shape.display == "none"
                || !is_inside_show_last(bar_index, outputs.len(), show_last)
                || !marker_is_active(shape.series, &shape.location)?
            {
                continue;
            }
            markers.push(IndicatorMarker::Plotshape {
                bar_index,
                value: shape.series,
                title: shape.title.clone(),
                text: shape.text.clone(),
                style: shape.style.clone(),
                location: shape.location.clone(),
                color: shape.color.as_ref().map(indicator_color),
                text_color: shape.textcolor.as_ref().map(indicator_color),
                size: shape.size.clone(),
                offset,
            });
        }

        for character in output.plotchars() {
            validate_marker_string(&character.title, MAX_INDICATOR_MARKER_TITLE_BYTES, "title")?;
            validate_marker_string(
                &character.char,
                MAX_INDICATOR_MARKER_CHARACTER_BYTES,
                "character",
            )?;
            validate_marker_string(&character.text, MAX_INDICATOR_MARKER_TEXT_BYTES, "text")?;
            validate_marker_location(&character.location)?;
            validate_marker_size(&character.size)?;
            let offset = validate_marker_offset(character.offset)?;
            let show_last = validate_show_last(character.show_last)?;
            if character.display == "none"
                || !is_inside_show_last(bar_index, outputs.len(), show_last)
                || !marker_is_active(character.series, &character.location)?
            {
                continue;
            }
            markers.push(IndicatorMarker::Plotchar {
                bar_index,
                value: character.series,
                title: character.title.clone(),
                character: character.char.clone(),
                text: character.text.clone(),
                location: character.location.clone(),
                color: character.color.as_ref().map(indicator_color),
                text_color: character.textcolor.as_ref().map(indicator_color),
                size: character.size.clone(),
                offset,
            });
        }

        for arrow in output.plotarrows() {
            validate_marker_string(&arrow.title, MAX_INDICATOR_MARKER_TITLE_BYTES, "title")?;
            let offset = validate_marker_offset(arrow.offset)?;
            let show_last = validate_show_last(arrow.show_last)?;
            ensure!(
                arrow.minheight.is_finite() && arrow.minheight >= 0.0,
                "plotarrow minheight must be finite and nonnegative"
            );
            ensure!(
                arrow.maxheight.is_finite() && arrow.maxheight >= 0.0,
                "plotarrow maxheight must be finite and nonnegative"
            );
            ensure!(
                arrow.minheight <= arrow.maxheight,
                "plotarrow minheight must not exceed maxheight"
            );
            if arrow.series.is_nan() {
                continue;
            }
            ensure!(arrow.series.is_finite(), "plotarrow value must be finite");
            if arrow.display == "none"
                || arrow.series == 0.0
                || !is_inside_show_last(bar_index, outputs.len(), show_last)
            {
                continue;
            }
            markers.push(IndicatorMarker::Plotarrow {
                bar_index,
                value: arrow.series,
                title: arrow.title.clone(),
                color_up: arrow.colorup.as_ref().map(indicator_color),
                color_down: arrow.colordown.as_ref().map(indicator_color),
                min_height: arrow.minheight,
                max_height: arrow.maxheight,
                offset,
            });
        }
    }
    Ok(IndicatorVisualData {
        version: INDICATOR_VISUAL_DATA_VERSION,
        markers,
    })
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
    let visual_data = collect_visual_data(&run.outputs)?;
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
        visual_data,
        diagnostics: validated.diagnostics,
    })
}

/// Run a validated indicator on a bounded blocking worker. The interpreter is
/// not process-isolated; the semaphore limits how much CPU a hostile script can consume.
pub async fn execute_indicator(
    execution_limit: Arc<Semaphore>,
    source: String,
    input_values: Value,
    candles: Arc<Vec<Candle>>,
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
            execute_blocking(
                &source,
                &input_values,
                candles.as_slice(),
                &symbol,
                &timeframe,
            )
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

    fn execute(source: &str) -> IndicatorExecutionOutput {
        execute_blocking(source, &serde_json::json!({}), &candles(), "BTC", "1m")
            .expect("execute test indicator")
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
                Arc::new(candles()),
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

    #[test]
    fn rejects_loops_before_metadata_execution() {
        for source in [
            "indicator(\"endless\")\nwhile true\n    continue",
            "indicator(\"large\")\nfor i = 0 to 1000000000\n    plot(i)",
            "indicator(\"collection\")\nfor item in array.from(1, 2)\n    plot(item)",
        ] {
            let error = validate_indicator_source(source, &serde_json::json!({}))
                .expect_err("loops must be rejected");
            assert_eq!(
                error.to_string(),
                "Pine loops are not supported by HyperVibes indicators"
            );
        }
    }

    #[test]
    fn collects_all_marker_kinds_with_native_metadata_and_order() {
        let output = execute(
            r#"//@version=5
indicator("Markers", overlay=true)
plotarrow(2.5, title="Momentum", colorup=color.green, colordown=color.red, minheight=4, maxheight=20, show_last=1)
plotarrow(-3, title="Negative", colorup=color.green, colordown=color.red, show_last=1)
plotchar(true, title="Stage 1", char="1", location=location.belowbar, color=color.yellow, text="Stage", textcolor=color.blue, size=size.small, offset=-1, show_last=1)
plotshape(close > 39, title="Buy", style=shape.triangleup, location=location.belowbar, color=color.green, text="BUY", textcolor=color.white, size=size.large, offset=2)
plotshape(0, title="Zero", location=location.absolute, show_last=1)
plotchar(0, title="Zero char", char="0", location=location.absolute, show_last=1)
"#,
        );

        assert_eq!(output.visual_data.version, INDICATOR_VISUAL_DATA_VERSION);
        assert_eq!(output.visual_data.markers.len(), 6);
        assert!(matches!(
            &output.visual_data.markers[0],
            IndicatorMarker::Plotshape {
                bar_index: 39,
                value,
                title,
                text,
                style,
                location,
                color: Some(IndicatorColor { green: 128, .. }),
                text_color: Some(IndicatorColor { red: 255, green: 255, blue: 255, .. }),
                size,
                offset: 2,
            } if *value == 1.0 && title == "Buy" && text == "BUY" && style == "triangleup"
                && location == "belowbar" && size == "large"
        ));
        assert!(matches!(
            &output.visual_data.markers[1],
            IndicatorMarker::Plotshape { bar_index: 39, value, location, .. }
                if *value == 0.0 && location == "absolute"
        ));
        assert!(matches!(
            &output.visual_data.markers[2],
            IndicatorMarker::Plotchar {
                bar_index: 39,
                character,
                text,
                text_color: Some(IndicatorColor { blue: 255, .. }),
                offset: -1,
                ..
            } if character == "1" && text == "Stage"
        ));
        assert!(matches!(
            &output.visual_data.markers[3],
            IndicatorMarker::Plotchar { bar_index: 39, value, character, location, .. }
                if *value == 0.0 && character == "0" && location == "absolute"
        ));
        assert!(matches!(
            &output.visual_data.markers[4],
            IndicatorMarker::Plotarrow {
                bar_index: 39,
                value,
                color_up: Some(IndicatorColor { green: 128, .. }),
                color_down: Some(IndicatorColor { red: 255, .. }),
                min_height,
                max_height,
                ..
            } if *value == 2.5 && *min_height == 4.0 && *max_height == 20.0
        ));
        assert!(matches!(
            &output.visual_data.markers[5],
            IndicatorMarker::Plotarrow { bar_index: 39, value, .. } if *value == -3.0
        ));
    }

    #[test]
    fn filters_inactive_hidden_and_show_last_markers() {
        let output = execute(
            r#"//@version=5
indicator("Filters")
plotshape(false, title="False")
plotshape(na, title="NA")
plotshape(true, title="Hidden shape", display=display.none)
plotchar(false, title="Hidden", display=display.none)
plotarrow(0, title="Zero")
plotarrow(na, title="NA arrow")
plotarrow(1, title="Hidden arrow", display=display.none)
plotshape(true, title="Last", show_last=2)
plotchar(true, title="Last char", char="3", show_last=2)
plotarrow(1, title="Last arrow", show_last=2)
"#,
        );

        assert_eq!(output.visual_data.markers.len(), 6);
        for marker in &output.visual_data.markers {
            let bar_index = match marker {
                IndicatorMarker::Plotshape { bar_index, .. }
                | IndicatorMarker::Plotchar { bar_index, .. }
                | IndicatorMarker::Plotarrow { bar_index, .. } => *bar_index,
            };
            assert!(bar_index >= 38);
        }
        assert!(matches!(
            output.visual_data.markers.as_slice(),
            [
                IndicatorMarker::Plotshape { .. },
                IndicatorMarker::Plotchar { .. },
                IndicatorMarker::Plotarrow { .. },
                IndicatorMarker::Plotshape { .. },
                IndicatorMarker::Plotchar { .. },
                IndicatorMarker::Plotarrow { .. },
            ]
        ));
    }

    #[test]
    fn validates_marker_limits_and_numeric_metadata() {
        let mut source = String::from("//@version=5\nindicator(\"Too many\")\n");
        for index in 0..11 {
            source.push_str(&format!("plotshape (true, title=\"S{index}\")\n"));
            source.push_str(&format!("plotchar (true, title=\"C{index}\")\n"));
            source.push_str(&format!("plotarrow (1, title=\"A{index}\")\n"));
        }
        let error = validate_indicator_source(&source, &serde_json::json!({}))
            .expect_err("combined marker call limit must be enforced");
        assert_eq!(
            error.to_string(),
            "indicator declares more than 32 marker output calls"
        );

        for offset in [f64::NAN, f64::INFINITY, 0.5, 2_001.0, -2_001.0] {
            assert!(validate_marker_offset(offset).is_err(), "offset {offset}");
        }
        for show_last in [Some(f64::NAN), Some(-1.0), Some(0.5)] {
            assert!(validate_show_last(show_last).is_err());
        }
    }

    #[test]
    fn caps_actual_per_candle_marker_outputs_and_rejects_invalid_arrow_heights() {
        fn arrow(minheight: f64, maxheight: f64) -> pine_lang::core::Plotarrow {
            pine_lang::core::Plotarrow {
                series: 1.0,
                title: String::new(),
                colorup: None,
                colordown: None,
                offset: 0.0,
                minheight,
                maxheight,
                editable: true,
                show_last: None,
                display: "all".to_string(),
                format: None,
                precision: None,
                force_overlay: false,
            }
        }

        let mut output = DefaultPineOutput::default();
        for _ in 0..=MAX_INDICATOR_MARKER_CALLS {
            output.add_plotarrow(arrow(5.0, 100.0));
        }
        assert!(collect_visual_data(&[output]).is_err());

        let mut output = DefaultPineOutput::default();
        output.add_plotarrow(arrow(10.0, 5.0));
        let error = collect_visual_data(&[output]).expect_err("invalid heights must fail");
        assert_eq!(
            error.to_string(),
            "plotarrow minheight must not exceed maxheight"
        );
    }

    #[test]
    fn empty_marker_output_uses_the_versioned_envelope() {
        let output = execute("//@version=5\nindicator(\"No markers\")\nplot(close)");
        assert_eq!(
            output.visual_data,
            IndicatorVisualData {
                version: INDICATOR_VISUAL_DATA_VERSION,
                markers: Vec::new(),
            }
        );
    }
}
