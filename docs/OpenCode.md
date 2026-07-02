# OpenCode Backend

Related docs:

- `README.md`
- `Agents.md`
- `Memory.md`
- `Hyperliquid.md`

## Goal

OpenCode is the only supported agent execution backend today.

Vibetrading owns:

- agent identity
- job scheduling and dispatch
- run tracking
- memory, account state, and order execution

OpenCode owns:

- LLM execution
- tool execution
- session lifecycle inside the runtime server

## Runtime Model

The runtime model is:

- one `agent_runtimes` row per OpenCode server instance
- one `agents.runtime_id` link from each agent to its runtime
- one generated workspace per agent

The seeded runtime is:

- `opencode-local` -> `OpenCode local` -> `http://localhost:14096`

`backend_kind` remains part of the schema even though only `opencode` is currently valid.

## Agent Workspaces

Creating an agent generates a workspace under the configured host workspace root.

New agent creation refuses to reuse an existing stale workspace directory for the same derived agent key.

The generated workspace includes:

- `opencode.json`
- `AGENTS.md`
- `.opencode/`
- `scripts/generated/`
- writable `scripts/user/`, `data/`, and `scratch/` paths

The workspace `.env` is backend-owned generated state and should not be read or modified by agents.

Re-generating a workspace refreshes generated files while preserving user-managed files under paths like `scripts/user/`, `data/`, and `scratch/`.

The agent settings page shows whether the on-disk workspace has drifted from `agent-runtime/workspace-template/` and lists changed template-managed files with per-file line counts.

That drift check only compares template-derived files such as `AGENTS.md`, `opencode.json`, `.opencode/...`, and `scripts/generated/...`. It does not inspect agent-created files under `scripts/user/`, `data/`, or `scratch/`.

Deleting an OpenCode agent deletes its generated workspace directory after the database delete succeeds.

## Scheduling

OpenCode jobs are scheduled by Vibetrading.

The `AgenticScheduler` claims due work, dispatches runs through the OpenCode backend adapter, and stores run state in Postgres.

Before claiming new work for an agent lane, Vibetrading reconciles stale active runs left behind by app restarts. A `running` run whose OpenCode session is recorded as `idle` after a command was created is marked `succeeded`; queued/running orphan rows that never reached OpenCode are failed after their configured timeout. This prevents one interrupted process from causing all later runs in the same lane to be skipped forever.

The agent detail page exposes a `Jobs` tab for OpenCode agents.

When a job is dispatched, Vibetrading builds the initial OpenCode command prompt with the agent metadata, selected instruments, strategy prompt, operator prompt, and trading account snapshot when applicable.
