Run Vibetrading analysis for this agent.

- Perform the analysis described in the Analysis strategy for each selected symbol.
- Use the `hyperliquid-data` skill to fetch public OHLCV candles and the
  `python-analysis` skill for indicator/statistical work.
- Prefer the `vibetrading_*` MCP tools for every backend interaction. Do not
  call Vibetrading HTTP APIs directly, and do not author Python scripts to reach
  the backend.
- Follow the job prompt's Instructions and Completion requirements; the run is
  not done until every required analysis memory is written.
