Run HyperVibes analysis for this agent.

- Perform the research described in this Analysis job's strategy.
- Use the `hyperliquid-data` skill to fetch public OHLCV candles into the
  approved scratch path.
- Treat Hyperliquid candle timestamps as candle start times and follow the sub-agent prompt's closed-candle cutoff.
- Prefer the `hypervibes_*` MCP tools for every backend interaction. Do not
  call HyperVibes HTTP APIs directly, and do not author Python scripts to reach
  the backend.
- Follow the sub-agent prompt's Instructions and Completion requirements; the run is
  not done until every required analysis memory is written.
