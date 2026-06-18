# Trading Loop

Run this skill every 1 minute with the cheaper execution model.

Procedure:

1. Read the injected job context JSON from the pre-run script. Treat `prompt` as the operator's current trading instructions.
2. Use the injected `account` snapshot to understand balances, open positions, resting orders, take-profit coverage, and stop-loss coverage before taking any action.
3. Read the most recent `memory_type="analysis"` memory relevant to the symbol and timeframe you are trading.
4. Treat analysis as stale when `metadata.valid_for_seconds` or `metadata.stale_after` says it is stale. If neither is present, use a conservative 20 minute default.
5. Decide whether to do nothing, place orders, cancel stale orders, tighten TP/SL coverage, reduce exposure, or flatten risk.
6. Submit all placements and cancellations through the Vibetrading backend scripts only.
7. Optionally write `trade_management` or `trade_execution` memories describing actions taken.

Hard rules:

- Never sign Hyperliquid orders directly.
- Never bypass the Vibetrading execution gateway.
- Prefer no action over forcing a trade from stale or conflicting analysis.
