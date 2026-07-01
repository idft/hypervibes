---
name: hyperliquid-data
description: Use when fetching Hyperliquid public OHLCV candle data in Vibetrading OpenCode workspaces, especially with fetch_ohlcv.py.
---

# Hyperliquid Data

Fetch OHLCV candle data directly from Hyperliquid's public REST API.

## Usage

Run the script from the agent workspace root:

```bash
python .opencode/skills/hyperliquid-data/fetch_ohlcv.py <SYMBOL> <TIMEFRAME> [--limit N] [--start-time EPOCH_MS] [--end-time EPOCH_MS]
```

`SYMBOL` and `TIMEFRAME` are positional arguments. Do not use unsupported flags
such as `--coin`, `--symbol`, `--timeframe`, or `--days`.

Examples:

```bash
python .opencode/skills/hyperliquid-data/fetch_ohlcv.py BTC 15m --limit 100
python .opencode/skills/hyperliquid-data/fetch_ohlcv.py ETH 1h --start-time 1710000000000 --end-time 1710100000000
```

The script writes candles to `scratch/ohlcv-cache/<SYMBOL>/<TIMEFRAME>/...json`
and prints a small JSON manifest to stdout. Read `output_path` from the manifest,
then load that file from `scripts/user/` code with pandas or the standard JSON
library. The shared analysis Python runtime already includes pandas and related
analysis libraries.

Use `--stdout` only for manual debugging. Normal agent analysis should use the
cached `output_path` so full candle data does not fill the LLM context window.

On each invocation, the script prunes only the cache directory for the requested
symbol and timeframe. Files older than `--ttl-hours` are removed; the default TTL
is 24 hours. Use `--no-prune` only for debugging.

## Environment

Set `HYPERLIQUID_ENVIRONMENT` to `mainnet` or `testnet`. Defaults to `mainnet`.

## Supported timeframes

`5m`, `15m`, `1h`, `4h`, `1d` (and any others Hyperliquid supports).

## Output

Small JSON manifest to stdout:

```json
{
  "symbol": "BTC",
  "timeframe": "15m",
  "candles": 100,
  "output_path": "scratch/ohlcv-cache/BTC/15m/20260701T120000Z-abc123.json",
  "ttl_hours": 24.0,
  "pruned_files": 0
}
```

The saved JSON file contains a compact array of candles. Each candle is returned
in Hyperliquid's native object shape.

Example pandas load:

```python
import json
import pandas as pd

with open("scratch/ohlcv-cache/BTC/15m/20260701T120000Z-abc123.json") as f:
    candles = json.load(f)

df = pd.DataFrame(candles)
```

Errors are printed to stderr and the script exits non-zero.
