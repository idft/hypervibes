# Agents Module

Related docs:

- `README.md`
- `OpenCode.md`
- `Memory.md`
- `Hyperliquid.md`

## Goal

The agents module is the system of record for:

- which trading agents exist
- which OpenCode workspace each agent owns
- which Hyperliquid account each agent controls
- which instruments each agent may trade
- which strategy prompts and API credentials belong to the agent

The agent API key is the ownership boundary for agent-facing account, memory,
and order operations. Hyperliquid signing keys are encrypted at rest and remain
server-owned.

## Current Data Model

The current implementation keeps the registry intentionally small:

- one `agents` row per trading agent
- one `agent_instruments` mapping table for selected markets
- one `agent_strategy_prompts` row per `(agent_key, prompt_kind)`

Important `agents` fields:

- `agent_key`
- `display_name`
- `enabled`
- `environment`
- `wallet_address`
- `api_key`
- `runtime_config`

Strategy prompts are no longer stored directly on `agents`. They live in `agent_strategy_prompts` with prompt kinds:

- `analysis`
- `market_analysis`
- `trading`
- `daily_review`

## OpenCode Execution

OpenCode is the only execution backend. Every agent uses the local OpenCode
server configured by `OPENCODE_BASE_URL`, which defaults to
`http://localhost:14096`.

`agents.runtime_config` stores per-agent workspace metadata. It does not store
or select an OpenCode server.

## Workspace Generation

Creating an OpenCode agent also generates a per-agent workspace.

Workspace generation completes before the agent registry row is inserted. If
workspace generation fails, no agent is persisted. Later initialization failures
also remove the newly created registry row and workspace.

Non-secret metadata is stored in `agents.runtime_config`, including:

- `workspace_host_path`
- `workspace_container_path`
- `profile_source`

The generated workspace `.env` receives the agent-scoped Vibetrading API key. The backend does not read that file back.

For OpenCode agents, the settings page also reports whether the generated workspace has drifted from `agent-runtime/workspace-template/`. The comparison is limited to template-managed files and ignores agent-authored files.

Analysis code ownership is separate from job execution. Analysis, market
analysis, trading, and daily review jobs may execute reusable analysis code but
must not modify `scripts/user/`. The disabled-by-default `analysis_coding`
hook is the only job allowed to change that tree, and it requires an explicit
strong provider/model selection.

Coding supports bootstrap and improvement runs. Generation occurs in an
isolated candidate workspace. A fixed MCP tool validates the candidate in the
shared analysis runtime and binds the result to its deterministic tree hash;
the worker promotes only those validated bytes. Failed validation or promotion
verification leaves the live analysis tree unchanged and can be rolled back
from retained versions.

The canonical entrypoint is `scripts/user/analyze.py`; supporting Python modules
are allowed and no model-owned manifest file is required. Reusable code produces
quantitative measurements and may produce calculation-derived indicator
signals. Analysis jobs combine those outputs with qualitative evidence and own
all final bias, confidence, actionability, and setup decisions. Candidate tests
are optional and should be focused on demonstrated bugs or nontrivial custom
math rather than duplicating the fixed contract validator.

The Hyperliquid fetch helper writes the analyzer's canonical input envelope
directly: `symbol`, `timeframe`, authoritative positive `interval_ms`, and
normalized OHLCV candles. Candle timestamps are open times. Analyzer code must
apply `timestamp_ms + interval_ms < boundary_ms`; candles closing exactly at the
boundary are excluded. Context mismatches and invalid intervals fail closed.

Trading normally consumes the latest `market_analysis` memory without market
data access. A memory may instead declare a finite conditional-execution
contract: selected confirmation timeframes and machine-readable analyzer rules.
Only then may trading fetch those closed candles and run the canonical analyzer.
It uses the resulting measurements only to verify the declared rules; it cannot
change the market-analysis thesis, levels, indicators, or execution plan.

Workspace regeneration is queued as agent-scoped maintenance work:

- regular re-generation preserves `scripts/user/`, `data/`, and `scratch/`
- hard reset deletes the full workspace before re-generating it
- only one queued/running maintenance task is allowed per agent
- duplicate regenerate submissions are rejected
- queued maintenance waits for active runs and OpenCode sessions marked `busy` or `retry` to finish

While workspace maintenance is queued or running:

- scheduled jobs are held until maintenance completes
- manual job `Run now` and manual hook `Run now` are blocked
- automatic follow-up hooks for already-running analysis jobs still complete before maintenance begins

Deleting an agent removes the registry row and cascades through agent-owned state:

- agent schedules, hooks, and runs
- memory records
- selected instruments
- Hyperliquid orders and order events
- account-scoped Hyperliquid sync/history rows for that wallet+environment

Global Hyperliquid instrument metadata is preserved.

## Instrument Selection

Each agent may be linked to zero or more Hyperliquid perp instruments.

- empty selection is allowed
- empty selection means the agent should not analyze markets or place new trades
- `POST /api/v1/orders` rejects orders for symbols not currently selected for that agent

OpenCode jobs receive the job-specific strategy prompt, the latest agent-level
learnings memory, selected instruments, and job metadata through the dispatched
prompt text. Trading jobs also receive a live account snapshot.
