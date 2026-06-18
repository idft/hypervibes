# Analysis Loop

Run this skill every 15 minutes with the stronger analysis model.

Procedure:

1. Read the injected job context JSON from the pre-run script. Treat `prompt` as the operator's current analysis instructions.
2. Fetch recent OHLCV directly from Hyperliquid with `/opt/data/plugins/vibetrading/.venv/bin/python /opt/data/plugins/vibetrading/scripts/fetch_ohlcv.py BTC 15m --limit 200`.
3. Optionally read recent `memory_type="analysis"` memories for the same symbol and timeframe with the bundled memory script.
4. Run technical analysis with `/opt/data/plugins/vibetrading/.venv/bin/python /opt/data/plugins/vibetrading/scripts/analyze_ohlcv.py --candles-json '...json...'.`
5. Produce exactly one analysis memory with:
   - `summary`: short one-line market takeaway
   - `content`: markdown narrative reasoning for humans
   - `metadata`: machine-readable handoff for the trading loop
6. Write exactly one `memory_type="analysis"` record through the Vibetrading backend.
7. Do not submit orders from this skill.

Analysis memory rules:

- Keep reserved machine-readable fields stable: `schema_version`, `analysis_kind`, `symbol`, `timeframe`, `generated_at`, `valid_for_seconds`, `bias`, `confidence`, `entry_setups`, `take_profit_levels`, `stop_loss_levels`, `invalidation`, `do_not_trade_if`, `source_memory_ids`, `extensions`.
- Put any strategy-specific extras only inside `metadata.extensions`.
- `content` should explain the reasoning clearly enough for an operator to read later.
- `metadata` should make the downstream trading decision easy to parse.
