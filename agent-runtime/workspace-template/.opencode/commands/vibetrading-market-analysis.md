Run Vibetrading market-analysis synthesis for this agent.

- Read the latest per-timeframe `analysis` memories for each selected symbol with `vibetrading_get_latest_analysis(symbol)`.
- Produce exactly one `market_analysis` memory per selected symbol.
- Use `vibetrading_write_memory` with `memory_type="market_analysis"` and omit the `timeframe` argument entirely. Do not pass `__omit__`, `none`, `null`, an empty string, or any other placeholder; the backend rejects a timeframe on market-analysis memories.
- Include source memory ids, source timeframes, schema version, analysis kind, and explicit validity metadata.
- If there is no actionable edge, write a neutral market analysis that tells trading not to open new exposure.
