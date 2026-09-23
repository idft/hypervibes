---
name: pine-indicators
description: Use when creating, modifying, or reviewing HyperVibes PineScript indicators, including plot, plotshape, plotchar, plotarrow, inputs, and indicator results.
---

# Pine Indicators

Author and improve server-side PineScript indicators that run on immutable,
server-fetched closed Hyperliquid candles. Indicators produce research evidence;
they do not place orders or manage positions.

## Workflow

1. Call `hypervibes_list_analysis_instruments` before creating or updating an
   indicator. Use the returned IDs unchanged as `instrument_ids`. If the list is
   empty, ask the operator to select Analysis instruments in Settings.
2. Before modifying an indicator, call `hypervibes_get_indicator` for its active
   source, inputs, immutable version ID, and targets. Inspect bounded runs with
   `hypervibes_get_indicator_results` when evidence from prior behavior matters.
3. Write one deterministic `indicator(...)` program using only the supported
   subset below. Conditions must use current or historical closed-candle data;
    never use future information or repainting assumptions.
   Choose one to eight explicit `timeframes`. The same source and input-value
   set runs independently on each timeframe. Use separate definitions when
   timeframe-specific inputs are required; this is not Pine `request.security`.
4. Create with `hypervibes_create_indicator`, or update with
   `hypervibes_update_indicator` and the active version ID as
   `expected_version_id`. An update creates a new immutable version.
5. Explain what the indicator measures, how to interpret its plots and markers,
   and the evidence for any Review-authored revision.

Chat mutations require operator confirmation. Review may mutate indicators only
when its run grants indicator-write capability. Loading this skill never grants
mutation authority.

## Supported Pine Surface

- Declare an indicator with `indicator("Name", overlay=true|false)`.
- Numeric outputs use `plot()` and remain candle-aligned time series.
- Visual outputs use `plotshape()`, `plotchar()`, and `plotarrow()`.
- Common calculations such as `ta.sma`, `ta.ema`, `ta.rsi`, `ta.macd`,
  `ta.crossover`, and `ta.crossunder` are available through the embedded Pine
  runtime.
- Supported inputs are boolean, integer, float, string, and price-source inputs.
  Input overrides are keyed by the Pine input title, not the variable name.
- Color inputs are not supported.
- `for`, `for in`, and `while` loops are prohibited. Imports, `request.*`,
  strategies, and libraries are also prohibited.
- Source is limited to 64 KiB, with at most 32 inputs, 32 numeric plots, and 32
  combined marker-output call sites.

Keep marker calls at global scope and put the condition or signed value in the
first argument. Do not place output calls inside conditional blocks or helper
functions.

## Shape Markers

Use `plotshape()` for boolean or numeric conditions:

```pine
//@version=5
indicator("EMA Signals", overlay=true)

fast = ta.ema(close, 9)
slow = ta.ema(close, 21)
buy = ta.crossover(fast, slow)
sell = ta.crossunder(fast, slow)

plot(fast, "Fast EMA")
plot(slow, "Slow EMA")
plotshape(buy, title="Buy", style=shape.triangleup, location=location.belowbar, color=color.green, text="BUY")
plotshape(sell, title="Sell", style=shape.triangledown, location=location.abovebar, color=color.red, text="SELL")
```

Supported styles are `xcross`, `cross`, `circle`, `triangleup`,
`triangledown`, `flag`, `arrowup`, `arrowdown`, `square`, `diamond`, `labelup`,
and `labeldown`.

## Character Markers

Use `plotchar()` when the visible character carries meaning:

```pine
plotchar(stage1, title="Stage 1", char="1", location=location.belowbar, color=color.green)
plotchar(stage2, title="Stage 2", char="2", location=location.belowbar, color=color.yellow)
plotchar(stage3, title="Stage 3", char="3", location=location.abovebar, color=color.red)
```

The character and nonempty annotation text are displayed together. Keep both
short and meaningful; do not use the title as a substitute for visible text.

## Arrow Markers

Use `plotarrow()` for a signed series:

```pine
impulse = fast - slow
plotarrow(impulse, title="Momentum", colorup=color.green, colordown=color.red)
```

Positive values produce up arrows, negative values produce down arrows, and
zero or `na` produces no marker. Arrow magnitude is retained in results, but the
chart uses a fixed arrow size.

## Marker Behavior

- Supported locations for shapes and characters are `abovebar`, `belowbar`,
  `top`, `bottom`, and `absolute`.
- `location.absolute` treats the first argument as the exact price, including
  zero. Other locations treat finite nonzero values as active conditions.
- Supported sizes are `auto`, `tiny`, `small`, `normal`, `large`, and `huge`.
- Positive and negative integral offsets are supported within stored candle
  history. A marker shifted outside available history is omitted.
- `show_last` limits source bars before offset is applied. `display.none`
  suppresses marker output.
- Marker text, color, size, and styles are deterministic approximations rather
  than pixel-perfect TradingView rendering. All markers attach to the
  candlestick series.

## Interpreting Results

`hypervibes_get_indicator_results` requires one configured timeframe and returns
bounded immutable runs for only that timeframe containing
closed candles, numeric plots, marker events, latest numeric values, diagnostics,
and run status. Marker events are not included in `latest_values`; inspect the
returned `markers` collection when evaluating signal timing.

Analysis receives exact frozen run evidence for each timeframe. A dependency
reported as `timed_out` is fixed for that Analysis run and must not be replaced
with a later completion. Chat and Review can pass `run_id` when an exact
historical result is needed for provenance.

Analysis should treat plots and markers as research evidence and publish its
conclusions as scoped memories. A marker is not an order instruction.

Review should compare signal timing with subsequent closed-candle outcomes and
actual trading results. Require enough observations, account for market regime,
and avoid optimizing around a few favorable examples. Modify an indicator only
when evidence supports a material improvement, and preserve useful behavior that
the evidence does not invalidate.
