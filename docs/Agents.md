# Agents Module

Related docs:

- `README.md`
- `OpenCode.md`
- `Memory.md`
- `Hyperliquid.md`

## Goal

The agents module is the system of record for:

- which trading agents exist
- which runtime each agent uses
- which Hyperliquid account each agent controls
- which instruments each agent may trade
- which strategy prompts and API credentials belong to the agent

## Current Data Model

The current implementation keeps the registry intentionally small:

- one `agents` row per trading agent
- one `agent_runtimes` row per reusable runtime instance
- one `agent_instruments` mapping table for selected markets
- one `agent_strategy_prompts` row per `(agent_key, prompt_kind)`

Important `agents` fields:

- `agent_key`
- `display_name`
- `enabled`
- `environment`
- `wallet_address`
- `api_key`
- `backend_kind`
- `runtime_id`
- `runtime_config`

Strategy prompts are no longer stored directly on `agents`. They live in `agent_strategy_prompts` with prompt kinds:

- `analysis`
- `market_analysis`
- `trading`
- `daily_review`

The Hyperliquid private key is encrypted before storage.

## Backend And Runtime Rules

Only `opencode` is currently valid for `backend_kind`.

The generic backend seam remains in place:

- `agent_runtimes.backend_kind`
- `agents.backend_kind`
- `agents.runtime_id`
- `agents.runtime_config`

This is intentional future-proofing in case another backend type is added later.

The initial seeded runtime is:

- `opencode-local` -> `OpenCode local` -> `http://localhost:14096`

## Workspace Generation

Creating an OpenCode agent also generates a per-agent workspace.

Non-secret metadata is stored in `agents.runtime_config`, including:

- `workspace_host_path`
- `workspace_container_path`
- `profile_source`

The generated workspace `.env` receives the agent-scoped Vibetrading API key. The backend does not read that file back.

For OpenCode agents, the settings page also reports whether the generated workspace has drifted from `agent-runtime/workspace-template/`. The comparison is limited to template-managed files and ignores agent-authored files.

Workspace regeneration is queued as agent-scoped maintenance work:

- regular re-generation preserves `scripts/user/`, `data/`, and `scratch/`
- hard reset deletes the full workspace before re-generating it
- only one queued/running maintenance task is allowed per agent
- duplicate regenerate submissions are rejected
- queued maintenance waits for active runs and live OpenCode sessions to finish

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

OpenCode jobs receive the job-specific strategy prompt, the latest agent-level learnings memory, selected instruments, and job metadata through the dispatched prompt text.
