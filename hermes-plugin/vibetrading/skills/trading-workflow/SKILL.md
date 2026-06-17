---
name: vibetrading:trading-workflow
description: Loop that turns Vibetrading plugin tools into a trading decision cycle.
---

# Vibetrading Trading Workflow

Use the Vibetrading plugin tools in this order. Stop at any step where the
answer is "do not trade" and return that decision.

1. `get_account_status` — know current risk / position state.
2. `fetch_candles` — pull market data for the chosen instrument and timeframe.
3. `analyze_market` — compute the requested indicators and form a view.
4. `read_memory` — load higher-timeframe context and prior plans for this
   symbol / timeframe.
5. Decide whether to trade. Be explicit about the thesis and the trigger.
6. If trading, call `place_order`. Pass `memory_record_ids` linking the
   order to the source memory (typically a `plan` or `hypothesis` record
   written in step 4 / 5).
7. `write_memory` — record the post-trade outcome, including entry price,
   status, and `group_id` if available.

If a leg is rejected or returns a non-resting status, prefer `cancel_order`
or `cancel_all` to flatten before retrying; the gateway is idempotent and
records per-leg errors in `order_events`.

Never bypass the Vibetrading execution gateway. Hyperliquid signing
belongs to the backend, not to this agent.
