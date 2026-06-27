Run Vibetrading analysis for this agent.

- Fetch OHLCV data for the currency specificed in the timeframe specified.
- Perform the analysis as described in in the Analysis strategy.
- You may write an execute any additional Python scripts as nessasary
- Prefer the `vibetrading` MCP tools for every backend interaction. Do not
  call Vibetrading HTTP APIs directly, and do not author Python scripts to
  reach the backend. 
- Write your final analysis using the  `write_memory` MCP tool when
  completed.
