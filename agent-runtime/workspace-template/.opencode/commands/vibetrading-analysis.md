Run Vibetrading analysis for this agent.

- Fetch OHLCV data for the selected currency and timeframe with
  `python .opencode/skills/hyperliquid-data/fetch_ohlcv.py`.
- Use the shared Python analysis runtime for pandas/numpy/statistics/technical
  indicator work. You may write and execute analysis-only Python scripts under
  `scripts/user/` as necessary.
- Perform the analysis described in the Analysis strategy.
- Prefer the `vibetrading` MCP tools for every backend interaction. Do not
  call Vibetrading HTTP APIs directly, and do not author Python scripts to reach
  the backend.
- Write your final analysis using the `write_memory` MCP tool when completed.
