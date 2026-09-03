---
name: hyperliquid-data
description: Use when fetching Hyperliquid public OHLCV candle data in HyperVibes OpenCode workspaces, especially with fetch_ohlcv.py.
---

# Hyperliquid Data

Fetch OHLCV candle data directly from Hyperliquid's public REST API.

## Usage

Run the script from the agent workspace root:

```bash
python .opencode/skills/hyperliquid-data/fetch_ohlcv.py <SYMBOL> <TIMEFRAME> [--limit N] [--start-time EPOCH_MS] [--end-time EPOCH_MS] [--closed-before EPOCH_MS] [--output-dir scratch/PATH]
```

`SYMBOL` and `TIMEFRAME` are positional arguments. Do not use unsupported flags
such as `--coin`, `--symbol`, `--timeframe`, or `--days`.

Examples:

```bash
python .opencode/skills/hyperliquid-data/fetch_ohlcv.py BTC 15m --limit 100 --closed-before 1783114200000 --output-dir scratch/ohlcv
python .opencode/skills/hyperliquid-data/fetch_ohlcv.py ETH 1h --start-time 1710000000000 --end-time 1710100000000
python .opencode/skills/hyperliquid-data/fetch_ohlcv.py BTC 15m --limit 500 --closed-before 1783114200000 --output-dir scratch/ohlcv
```

The script writes candles to `scratch/ohlcv/` by default and prints a small JSON
manifest to stdout. Use `--output-dir` to select another directory under
`scratch/`; absolute paths and paths outside `scratch/` are rejected. Read
`output_path` from the manifest, then pass that file directly to the package
script you choose under `scripts/user/` with its `--input` argument. Do not
modify package code or create a helper script.

Use `--stdout` only for manual debugging. Normal agent analysis should use the
run-local `output_path` so full candle data does not fill the LLM context window.

## Environment

Set `HYPERLIQUID_ENVIRONMENT` to `mainnet` or `testnet`. Defaults to `mainnet`.

## Supported timeframes

`1m`, `3m`, `5m`, `15m`, `30m`, `1h`, `2h`, `4h`, `8h`, `12h`, `1d`,
`3d`, `1w`, and `1M`.

## Closed Candles

- Hyperliquid candle field `t` is the candle start time.
- A `15m` candle with `t = 21:30:00Z` spans `21:30:00Z` through `21:44:59.999Z` and is not closed at `21:30:00Z`.
- A candle is fully closed at `start + interval_ms`. A candle that only opened
  before a boundary has not closed at the boundary and must be excluded to
  avoid leaking unclosed data into deterministic analysis.
- For a sub-agent anchored to boundary `B`, always fetch with
  `--closed-before B_ms`. The script derives the candle close timestamp
   locally from `start + interval_ms` (the Hyperliquid API omits a reliable
   close field) and keeps candles whose close is at or before `B_ms`. This
   matters for shorter jobs fetching longer-timeframe data:
  a 15-minute sub-agent at the half-hour boundary `B` requesting 1-hour candles
   includes the 1-hour candle that opened at `B - interval_ms`, because it
   closes exactly at `B`, but excludes later candles.
- Do not pair `--end-time` with `--closed-before`; the local close filter
   is what guarantees no future-leakage.
- With `--closed-before`, `--limit` is the minimum number of eligible candles.
  The helper expands its fetch window when needed, then saves the most recent
  eligible candles. It fails rather than returning fewer than requested.

## Output

Small JSON manifest to stdout:

```json
{
  "symbol": "BTC",
  "timeframe": "15m",
  "interval_ms": 900000,
  "candles": 100,
  "output_path": "scratch/ohlcv/20260701T120000Z-abc123.json",
  "requested_boundary_ms": 1783114200000,
  "actual_max_close_ms": 1783114200000
}
```

`requested_boundary_ms` is `null` when `--closed-before` is not supplied;
`actual_max_close_ms` is `null` when no candles pass the filter.

The saved JSON file is the canonical input for the package script you choose.
Pass the manifest's `output_path` directly to that program's `--input`
argument. Its shape is:

```json
{
  "symbol": "BTC",
  "timeframe": "15m",
  "interval_ms": 900000,
  "candles": [
    {
      "timestamp_ms": 1783112400000,
      "open": 108000.0,
      "high": 108100.0,
      "low": 107900.0,
      "close": 108050.0,
      "volume": 12.5
    }
  ]
}
```

`interval_ms` is authoritative. Analyzer code must reject a missing or
non-positive interval rather than infer cadence from candle spacing.

Errors are printed to stderr and the script exits non-zero.
