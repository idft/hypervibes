Run Vibetrading analysis for this agent.

- Fetch OHLCV data for the selected currency and timeframe with
  `python .opencode/skills/hyperliquid-data/fetch_ohlcv.py <SYMBOL> <TIMEFRAME> [--limit N]`.
  `SYMBOL` and `TIMEFRAME` are positional arguments; do not use `--coin`,
  `--timeframe`, or `--days`. The command prints a small manifest and writes
  candles to `scratch/ohlcv-cache/<SYMBOL>/<TIMEFRAME>/...json`; use the
  manifest's `output_path` in analysis scripts.
- Use the shared Python analysis runtime for pandas/numpy/statistics/technical
  indicator work. You may write and execute analysis-only Python scripts under
  `scripts/user/` as necessary.
- Perform the analysis described in the Analysis strategy.
- Prefer the `vibetrading_*` MCP tools for every backend interaction. Do not
  call Vibetrading HTTP APIs directly, and do not author Python scripts to reach
  the backend.
- Do not stop after planning, skill loading, or candle fetch alone.
- The job is incomplete until `vibetrading_write_memory` succeeds for each
  selected symbol.
- Write exactly one timeframe-specific memory per selected symbol with
  `memory_type="analysis"` and the job timeframe.
- If there is no actionable setup, still write a neutral analysis memory that
  explicitly says there is no trade.
- If you use `todowrite`, end with no items left `pending` or `in_progress`.
