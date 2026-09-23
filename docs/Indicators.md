---
slug: /indicators
---

# Indicators

HyperVibes indicators are deterministic PineScript-subset programs evaluated
against server-fetched, closed Hyperliquid candles. They produce research
evidence and cannot place orders.

## Definitions, versions, and runs

An indicator definition owns its name, enabled state, selected Analysis
instruments, and one to eight explicit timeframes. Timeframes are normalized,
unique, and ordered by duration. One source version and one input-value set
apply to every configured timeframe; create separate definitions when inputs
must differ by timeframe. Multiple configured timeframes are independent runs,
not Pine `request.security` calls.

Every source change creates an immutable version. A run snapshots one version,
instrument, timeframe, and canonical closed-candle boundary. Its durable
identity is `(version, instrument, timeframe, boundary)`, so results from
different candle series cannot collide. Creating or updating an enabled
definition immediately queues the active version at the latest closed boundary
for every configured timeframe and selected instrument. The scheduler then
reconciles only the latest boundary; it does not backfill downtime.

For example, a definition targeting BTC with `15m`, `1h`, and `4h` creates
three independent BTC targets. At 12:00 UTC their canonical boundaries are all
12:00, but at 12:15 only the `15m` target advances. Charts and result reads
require a timeframe and never combine those series.

## Analysis dependencies

Each Analysis run persists its applicable indicator set before dispatch. For an
Analysis boundary `B`, every indicator timeframe uses its greatest canonical
close at or before `B`. Thus Analysis at 10:15, 10:30, and 10:45 can all reuse
the same `1h` run at 10:00; Analysis at 11:00 uses the 11:00 run. Definition,
version, instrument, timeframe, boundary, and run ID remain frozen even if the
indicator configuration changes later. A set is limited to 1,024 dependencies.

The persisted deadline is not reset by a restart. At the deadline, queued or
running dependencies freeze as `timed_out`; later completion does not make them
visible to that Analysis credential. In-process completion notifications reduce
latency, while database polling remains the restart and multi-process fallback.

## Execution and retries

Indicator workers use leased, token-fenced claims. A stale worker cannot write
after another attempt owns its run. Retryable infrastructure and temporary
market-data failures use persisted exponential backoff and make at most three
attempts. Invalid timeframes, source or input errors, compilation/runtime
failures, non-finite output, and result-size violations are terminal.

Successful runs retain the exact bounded candle input, numeric plots, visual
marker data, latest values, and diagnostics used for the result.

At reconciliation cadence, structured debug logging reports ready depth, oldest
ready age, running and retry counts, and frozen timed-out dependency count.

## Configuration

- `INDICATOR_MAX_CONCURRENT_EXECUTIONS` optionally sets `1..=32` workers per
  application process. Without it, HyperVibes uses one less than the detected
  logical CPU count, with a minimum of 1 and maximum of 32. Multi-replica
  deployments must divide host capacity explicitly.
- `INDICATOR_ANALYSIS_WAIT_TIMEOUT_SECONDS` accepts `1..=300` and defaults to
  30 seconds. It is persisted per Analysis dependency set; unfinished work is
  frozen as `timed_out` and remains unavailable to that dispatch.

Definitions are archived rather than deleted so immutable runs can remain
historical evidence. Retention never removes a run referenced by an Analysis
dependency. Review and Chat may request a historical result by exact `run_id`;
Analysis credentials can read only succeeded, failed, or skipped runs in their
own frozen dependency set.

The operator UI supports the same one-to-eight timeframe contract as the JSON
API and MCP tools. The API uses only the plural `timeframes` field for
definition mutations.
