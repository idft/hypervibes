---
name: python-analysis
description: Use when writing Python analysis scripts in Vibetrading OpenCode workspaces, especially for pandas, numpy, pandas-ta-classic, statistics, plotting, or OHLCV analysis.
---

# Python Analysis

Use the container-provided Python analysis runtime for market analysis scripts.

## Runtime

Run scripts with `python`. The `python` executable is provided by the shared
analysis virtualenv at `/opt/vibetrading/analysis/.venv`.

Common libraries available:

- `requests`, `httpx`
- `numpy`, `pandas`, `scipy`, `statsmodels`
- `pandas_ta_classic` for technical indicators
- `polars`
- `matplotlib`, `seaborn`, `plotly`
- `tabulate`

## Writable Paths

Write reusable analysis scripts under `scripts/user/`.

Write durable analysis outputs under `data/`.

Use `scratch/` for temporary files.

Do not write generated helper scripts outside those paths.

## Backend Boundary

Do not write Python scripts that call Vibetrading HTTP APIs directly. Use the
`vibetrading` MCP tools for backend access, memory reads/writes, account state,
and orders.

It is allowed to fetch public market data directly from Hyperliquid using the
canonical Hyperliquid data skill script.

## OHLCV Workflow

Fetch candles with:

```bash
python .opencode/skills/hyperliquid-data/fetch_ohlcv.py <SYMBOL> <TIMEFRAME> --limit <N>
```

`SYMBOL` and `TIMEFRAME` are positional arguments. Do not use unsupported flags
such as `--coin`, `--timeframe`, or `--days`.

The command prints a small manifest to stdout and writes candle JSON to
`scratch/ohlcv-cache/<SYMBOL>/<TIMEFRAME>/...json`. Read the manifest's
`output_path`, then load that JSON file into pandas for analysis. Do not use
`--stdout` unless manually debugging; full candle stdout can fill the LLM
context window.

Example import for technical indicators:

```python
import pandas as pd
import pandas_ta_classic as ta
```
