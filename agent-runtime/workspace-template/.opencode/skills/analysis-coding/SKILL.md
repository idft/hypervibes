---
name: analysis-coding
description: Use only for the analysis-coding sub-agent to modify the canonical reusable analysis implementation.
---

# Analysis Coding

This skill defines the stable interface for `scripts/user/`. Use it only in an
isolated coding candidate workspace through the available MCP tools.

## Canonical Interface

The entrypoint is `scripts/user/analyze.py`:

```text
python scripts/user/analyze.py \
  --symbol BTC \
  --timeframe 15m \
  --boundary-ms 1700000900000 \
  --input scratch/runs/input.json \
  --output scratch/runs/analysis.json
```

Input JSON is written by the canonical `hyperliquid-data/fetch_ohlcv.py` helper.
It contains `symbol`, `timeframe`, a positive authoritative `interval_ms`, and
normalized candles with `timestamp_ms`, `open`, `high`, `low`, `close`, and
`volume`. CLI and input symbol/timeframe must agree.

`timestamp_ms` is the candle's open time. Its complete close time is
`timestamp_ms + interval_ms`. A candle is eligible only when:

```text
timestamp_ms + interval_ms < boundary_ms
```

A candle closing exactly at the boundary is excluded. Apply this rule before
every calculation. Never infer cadence from candle spacing, and never fall back
to comparing the open timestamp alone. Reject missing/invalid intervals,
context mismatches, unsupported input, and calculation failures with a non-zero
exit instead of returning a successful empty measurement set.

Write deterministic JSON atomically. The required output shape is:

```json
{
  "symbol": "BTC",
  "timeframe": "15m",
  "boundary_ms": 1700000900000,
  "source_range": {
    "count": 1
  },
  "code_version": "analyze.py@1",
  "measurements": {
    "last_close": 101.0
  },
  "warnings": []
}
```

`source_range.count` is mandatory and is the exact number of eligible candles
used for calculations. Additional source-range fields are allowed only when
they describe eligible data; never emit total input counts or metadata that
changes when an ineligible open/future candle is appended. Measurements must be
non-empty, finite, deterministic, and sensitive to eligible candle values. Sort
eligible candles by `timestamp_ms` before calculations so output is independent
of input order. The implementation must work with only one eligible candle and
with every canonical timeframe by using the input's authoritative
`interval_ms`.

The output may contain `signals` for calculation-derived indicator states or
events. Signals must include enough measurements or thresholds to audit their
result; they must not contain trading policy or order instructions. Signal names
must match their calculations. In particular, a `last_candle_body` state is
`up`, `down`, or `flat` by comparing that candle's close with its open, not by
comparing its close with the previous candle.

## Bootstrap Procedure

In bootstrap mode, first implement the smallest robust canonical baseline. A
finite eligible-candle count and last-close measurement are sufficient. Do not
attempt to implement every indicator or every item in the analysis strategy in
the initial bootstrap. Later improvement jobs can add evidence-supported
analytics after the stable interface passes validation.

The fixed validator supplies local deterministic fixtures; it does not need
network data, a fetched input, or files under `scratch/`. It checks one-candle
operation, every supported interval, strict boundary exclusion, context
mismatch rejection, deterministic output, finite non-empty measurements,
eligible-candle sensitivity, input-order invariance, known signal semantics,
and creation of a missing parent directory for the requested atomic output.

Run fixed validation after the baseline is complete. Treat its result as
authoritative:

- If `ok` is `false`, use the reported check and diagnostics to fix the
  candidate, then validate again.
- Never dismiss a validation failure as environmental or claim local tests
  prove the candidate is valid.
- Submit the coding report only after validation returns `ok: true` for
  the exact final tree.
- Report changed paths relative to the `scripts/user` root, for example
  `analyze.py` and `tests/test_indicator.py`, not `scripts/user/analyze.py`.

## Constraints

- In bootstrap mode, create the canonical `analyze.py` if it is absent. A
  bootstrap run must not return `no_change` merely because the tree is empty.
- Preserve this CLI and schema; extend fields without renaming required ones.
- Supporting modules under `scripts/user` are allowed.
- Use focused edits and Pyright LSP diagnostics; do not replace a whole
  large file when a local edit is enough. Resolve every reported Pyright error
  before final validation.
- Production code may use NumPy, pandas, SciPy, statsmodels,
  pandas-ta-classic, and Polars from the preinstalled analysis runtime.
- Tests are optional. Add a focused standard-library `unittest` regression for
  a demonstrated bug or nontrivial custom quantitative calculation; do not
  recreate the platform contract suite. A minimal bootstrap using simple
  filtering and measurements does not need candidate tests. Never create
  temporary diagnostic or placeholder files.
- Quantitative signals may describe crossovers, threshold events, breakouts,
  divergences, or custom indicator states. Do not emit final trade direction,
  actionability, trading confidence, entries, exits, stops, targets, sizing, or
  orders.
- Do not call HyperVibes APIs, place orders, install packages, or depend on
  agent-specific absolute paths.
- Run the fixed validation tool until it returns `ok: true`, then submit exactly
  one structured coding report when finished.
