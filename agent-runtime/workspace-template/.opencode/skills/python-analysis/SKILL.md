---
name: python-analysis
description: Use during an analysis job to run existing quantitative tools against canonical OHLCV input without modifying reusable code.
---

# Python Analysis

Use the container-provided immutable Python analysis runtime for existing
quantitative tools. This skill does not authorize creating, editing, or running
new Python scripts.

## Runtime

The `python` executable is provided by the shared analysis virtualenv at
`/opt/hypervibes/analysis/.venv`.

The analysis-coding sub-agent owns reusable implementation under `scripts/user/`
through its separate `analysis-coding` skill. Analysis jobs may only invoke the
existing canonical analyzer and use its output as evidence.

## Inputs and Outputs

Use `hypervibes_run_analysis_tool` to invoke a tool declared by
`scripts/user/manifest.json`. Pass canonical OHLCV input and output paths under
the role-approved `scratch/` path. The platform validates the output and binds
it to the package and tool versions. Do not create temporary helper scripts,
write durable `data/` outputs, or install packages.

## Backend Boundary

Do not call HyperVibes HTTP APIs directly. Use the `hypervibes` MCP tools for
backend access, memory reads/writes, account state, and orders.

Public market data may be fetched only through the canonical Hyperliquid data
skill script.

## OHLCV Workflow

Fetch candles with:

```bash
python .opencode/skills/hyperliquid-data/fetch_ohlcv.py <SYMBOL> <TIMEFRAME> --limit <N>
```

`SYMBOL` and `TIMEFRAME` are positional arguments. Do not use unsupported flags
such as `--coin`, `--timeframe`, or `--days`.

The command prints a small manifest to stdout. Read its `output_path`, then pass
it directly to the declared analysis tool. Do not use
`--stdout`; full candle stdout can fill the LLM context window.
