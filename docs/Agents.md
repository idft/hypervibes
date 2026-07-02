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
- which prompts and API credentials belong to the agent

## Current Data Model

The current implementation keeps the registry intentionally small:

- one `agents` row per trading agent
- one `agent_runtimes` row per reusable runtime instance
- one `agent_instruments` mapping table for selected markets

Important `agents` fields:

- `agent_key`
- `display_name`
- `enabled`
- `environment`
- `wallet_address`
- `api_key`
- `analysis_prompt`
- `trading_prompt`
- `backend_kind`
- `runtime_id`
- `runtime_config`

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

OpenCode jobs receive agent prompts, selected instruments, and job metadata through the dispatched prompt text.
