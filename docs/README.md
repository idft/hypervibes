# Vibetrading

Vibetrading runs OpenCode-backed crypto trading agents on Hyperliquid. It is a
Rust application with an operator UI, a scoped agent API, Postgres persistence,
and supervised background work for agent jobs and exchange monitoring.

## Documentation

- `Architecture.md`: runtime, subsystem boundaries, configuration, and UI
- `Agents.md`: agent registry, credentials, prompts, instruments, and workspaces
- `OpenCode.md`: OpenCode runtime, workspace, scheduling, and dispatch behavior
- `Memory.md`: append-only agent memory and retrieval contracts
- `Hyperliquid.md`: exchange synchronization, account journal, and order gateway
- `WalletSecurity.md`: wallet-login, agent-wallet, approval, and session security model
- `Testing.md`: test database and frontend/SSE test behavior

## Source Map

- `src/main.rs`: application startup and shutdown
- `src/web/`: operator UI, SSE, and `/api/v1` agent API
- `src/harness/`: unified OpenCode jobs, runs, maintenance, and dispatch
- `src/agents/`: agent registry, credentials, and Hyperliquid monitor
- `src/hyperliquid/`: exchange integration and execution gateway
- `src/memory/`: memory persistence and queries
- `agent-runtime/`: generated OpenCode workspace template and MCP adapter
- `migrations/`: authoritative database schema

When documentation and implementation disagree, treat the source and
migrations as authoritative and update these documents with the confirmed
behavior.
