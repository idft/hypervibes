/// Global system prompt prepended to every OpenCode agent job prompt.
pub const SYSTEM_PROMPT: &str =
    "This is the HyperVibes system for trading perpetual futures on Hyperliquid.";

/// Default analysis strategy prompt (the editable "user prompt" describing the
/// strategy). This is an example format; operators may replace it entirely with
/// their own structure, which is passed verbatim to the LLM.
pub const DEFAULT_ANALYSIS_STRATEGY_PROMPT: &str = "## Strategy overview\nConservative swing trading. Trade in the direction of the higher-timeframe trend and favor continuation over counter-trend reversals.\n\n## Markets & timeframes\nAnalyze this job's timeframe, always framed against higher-timeframe context (trend and key levels).\n\n## What to analyze\nMarket structure, trend and momentum, volatility regime, support/resistance, liquidity zones, and risk/reward.\n\n## Bias & actionable criteria\nClassify the bias as long, short, neutral, or mixed. A setup is only actionable when there is a clear directional edge with a defined entry zone, a logical invalidation level, and at least one target.\n\n## Confidence & validity\nMinimum confidence for an actionable setup: 0.65. Default analysis validity: 15 minutes; shorten it when volatility is high or the setup hinges on a level that may break soon.\n\n## No-trade discipline\nIf there is no clear edge, mark the bias neutral or mixed and provide no actionable setup. Do not manufacture setups.";

/// Default market-analysis strategy prompt. This is intentionally separate
/// from the timeframe analysis prompt because this job synthesizes prior
/// analyses into an execution-facing view.
pub const DEFAULT_MARKET_ANALYSIS_STRATEGY_PROMPT: &str = "## Synthesis goal\nTurn the latest timeframe analyses into one execution-facing market view per symbol. Preserve the strongest shared directional thesis and discard stale or conflicting details.\n\n## Decision standard\nOnly produce an actionable market analysis when the source analyses agree on direction, levels, and invalidation well enough for execution. Otherwise write a neutral market analysis that explicitly blocks new exposure.\n\n## Output focus\nSummarize bias, confidence, entry zone, invalidation, targets, and the concrete reasons trading should or should not act now.\n\n## Summary title\nUse a concise summary title of no more than 12 words. Include the symbol, directional bias or no-trade status, and the key reason or next step. Never include a date, time, timestamp, timeframe, or other metadata in the title.\n\n## Memory hygiene\nCarry forward the source analysis memory IDs and timeframes in metadata so the trading and review jobs can trace how the synthesis was formed.";

/// Default trading strategy prompt (the editable "user prompt" describing the
/// trading configuration). This is an example format; operators may replace it
/// entirely with their own structure, which is passed verbatim to the LLM.
pub const DEFAULT_TRADING_STRATEGY_PROMPT: &str = "## Position sizing\nOpen small starter positions; never take a full-size position on the first fill. Scale in only on confirmation, never to average down a losing position.\n\n## Entry strategy\nUse 1-3 resting limit orders laddered across the setup's entry zone. Weight size toward stronger levels and use smaller size on lower-confidence setups. Time-in-force: normal resting limit (gtc) unless the setup explicitly requires maker-only (alo).\n\n## Stop loss (strongly recommended)\nAttach a stop-loss leg to every entry at (or just beyond) the analysis invalidation level. Do not omit it unless the operator explicitly directs an unprotected order.\n\n## Take profit (strongly recommended)\nAttach take-profit leg(s) at the analysis targets. Prefer scaling out (partial take-profit at the first target, hold or trail a runner) over all-or-nothing exits.\n\n## Risk limits\nRequire a minimum reward:risk of 1.5:1 after fees; skip setups that do not clear it.\n\n## Confidence gate\nOnly open new exposure when the market analysis is fresh, non-neutral, and confidence is at least 0.65. Do not add exposure when analysis is neutral, mixed, stale, or blocked.\n\n## Order management\nCancel unfilled entry orders when the source analysis expires or is invalidated. Avoid duplicate resting orders at similar prices.";

/// Default daily-review strategy prompt.
pub const DEFAULT_DAILY_REVIEW_STRATEGY_PROMPT: &str = "## Review goal\nReview the agent's recent analyses, market analyses, orders, and learnings for one UTC day. Focus on whether decisions matched the evidence available at the time.\n\n## What to look for\nIdentify good patterns, repeated mistakes, stale assumptions, execution failures, risk-control failures, and places where prompts should be improved.\n\n## Learning standard\nOnly write new agent-level learnings when something materially changed in what the agent should remember going forward. Each new `agent_learnings` memory is the complete current canonical learning set: carry forward still-valid prior learnings, add the new learning, and explicitly remove or replace superseded rules. Use the fixed summary `Accumulated agent learnings`. Keep learnings durable, concise, and general enough to reuse across future sessions.\n\n## Workspace edits\nDaily review is a diagnosis job. Never modify `scripts/user/`, `data/`, `scratch/`, or analysis helper code in any way. If review evidence suggests the analysis code should change, surface that request in your review metadata under `analysis_coding_requested` and explain specifically what improvement would help; the separate analysis-coding job owns all such code changes.";

/// Default analysis-coding strategy prompt. Coding owns reusable
/// quantitative code, while analysis jobs retain responsibility for market
/// interpretation and trading conclusions.
pub const DEFAULT_ANALYSIS_CODING_STRATEGY_PROMPT: &str = r#"## Role
Improve the agent's reusable quantitative analysis code under `scripts/user` only when the analysis strategy or accumulated evidence supports a change.

## Required survey
- Inspect the current analysis strategy for calculations or custom indicators that reusable code can support.
- Read the latest accumulated agent learnings.
- Read the existing candidate files before creating or replacing code.

## Quantitative boundary
Produce auditable measurements and deterministic quantitative signals. A signal may report an indicator state, threshold crossing, crossover, breakout, divergence, or other calculation-derived event. Include the measurements and thresholds needed to audit it.

Do not generate trading policy. Reusable code must not decide final long/short bias, actionability, trading confidence, entries, exits, stops, targets, position sizing, or order instructions. Analysis jobs combine quantitative output with qualitative market evidence and decide what it means.

## Stable interface
Keep `scripts/user/analyze.py` as the canonical entrypoint with `--symbol`, `--timeframe`, `--boundary-ms`, `--input`, and `--output`. Supporting modules under `scripts/user` are allowed. Improve the stable implementation instead of creating suffixed duplicate entrypoints.

The canonical input is the JSON envelope written by `fetch_ohlcv.py`: `symbol`, `timeframe`, positive `interval_ms`, and normalized `candles` containing `timestamp_ms`, `open`, `high`, `low`, `close`, and `volume`. Validate that CLI and input symbol/timeframe agree. Treat `interval_ms` as authoritative; never infer cadence from candle spacing.

## Evidence discipline
Prefer a small correct implementation over a broad strategy engine. Make evidence-supported changes and return `no_change` when no reusable quantitative improvement is justified.

## Dependencies
Production code may use the immutable analysis runtime, including NumPy, pandas, SciPy, statsmodels, pandas-ta-classic, and Polars. Prefer established library calculations over handwritten indicator implementations. Never install or modify packages.

## Tests
Candidate tests are optional. Add a focused standard-library `unittest` regression when fixing a demonstrated bug or implementing nontrivial custom quantitative math. Do not recreate the platform contract tests or generate a comprehensive suite by default.

## Future-data discipline
Every candle path must exclude candles whose complete close time is not strictly before `--boundary-ms`. Candle timestamps are start times, so use the requested timeframe duration when determining close time. Treat the boundary as authoritative.

The exact eligibility rule is `timestamp_ms + interval_ms < boundary_ms`; a candle closing exactly at the boundary is excluded. Reject invalid context, missing intervals, unsupported input, and calculation failures with a non-zero exit rather than emitting a successful empty measurement set.

## Safety
Use Pyright LSP diagnostics for Python. Never call HyperVibes APIs, place or cancel orders, edit prompts, access another workspace, or hard-code agent-specific paths. Run fixed validation and submit exactly one structured report."#;
