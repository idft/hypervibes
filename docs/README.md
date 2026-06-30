# Vibetrading V2

Vibetrading is a Rust application for running OpenCode-backed trading agents on Hyperliquid.

Current direction:

- one main application binary
- Postgres as the primary database
- one shared migrations directory
- dedicated `agents`, `memory`, and `hyperliquid` database areas
- a server-rendered operator UI plus internal agent APIs
- supervised background tasks for account monitoring and OpenCode job dispatch

Current docs:

- `Agents.md` for agent registry, runtimes, prompts, and instruments
- `OpenCode.md` for the supported agent backend
- `Memory.md` for memory storage and retrieval
- `Hyperliquid.md` for venue sync and execution ownership
- `Architecture.md` for the current runtime shape

Notes:

- `agent_runtimes.backend_kind` remains in the schema even though only `opencode` is currently valid.
- `/api/v1/job-context` still exists temporarily for older runtime flows, but it is deprecated.
