---
name: python-analysis
description: Use during an analysis job to inspect and directly execute the read-only Coding package under scripts/user/ without modifying it.
---

# Python Analysis

Use the container-provided immutable Python analysis runtime to inspect and
directly execute the copied Coding package. This skill does not authorize
creating, editing, or replacing any package code.

## Runtime

The `python` executable is provided by the shared analysis virtualenv at
`/opt/hypervibes/analysis/.venv`.

The analysis-coding sub-agent owns the reusable package under `scripts/user/`
through its separate `analysis-coding` skill. Analysis jobs may read and
directly execute any package Python file, and use the results as evidence.

## The Coding Package

The copied Coding package under `scripts/user/` is read-only reusable agent
code. It may contain any number of strategies, entrypoints, and helper
modules; inspect the tree and the package manifest to find the script that
fits the job. Directly execute package Python as needed with the shared
analysis runtime, for example:

```bash
python scripts/user/strategies/trend.py --symbol BTC --timeframe 15m \
  --boundary-ms 1700000900000 --input scratch/ohlcv/input.json \
  --output scratch/analysis-output/trend.json
```

Write any transient inputs and outputs only under the approved run-local
`scratch/` directories. Do not create replacement or temporary package code
during an analysis run, do not write durable `data/` outputs, and do not
install packages.

## Backend Boundary

Do not call HyperVibes HTTP APIs directly. Use the `hypervibes` MCP tools for
backend access, memory reads/writes, account state, and orders.

Public market data may be fetched only through the canonical Hyperliquid data
skill script.

## OHLCV Workflow

Fetch candles with:

```bash
python .opencode/skills/hyperliquid-data/fetch_ohlcv.py <SYMBOL> <TIMEFRAME> --limit <N> --closed-before <BOUNDARY_MS> --output-dir scratch/ohlcv
```

`SYMBOL` and `TIMEFRAME` are positional arguments. Do not use unsupported flags
such as `--coin`, `--timeframe`, or `--days`.

The command prints a small manifest to stdout. Read its `output_path`, then run
the package script you chose directly with that file as the `--input`
argument. Do not use `--stdout`; full candle stdout can fill the LLM context
window.