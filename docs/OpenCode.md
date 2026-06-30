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

The generated workspace includes:

- `opencode.json`
- `AGENTS.md`
- `.opencode/`
- `scripts/generated/`
- writable `scripts/user/`, `data/`, and `scratch/` paths

The workspace `.env` is backend-owned generated state and should not be read or modified by agents.

## Scheduling

OpenCode jobs are scheduled by Vibetrading.

The `AgenticScheduler` claims due work, dispatches runs through the OpenCode backend adapter, and stores run state in Postgres.

The agent detail page exposes a `Jobs` tab for OpenCode agents.

## Deprecated API Note

`/api/v1/job-context` still exists temporarily for older flows, but new OpenCode job dispatch should prefer prompt-injected context and the MCP tools.
